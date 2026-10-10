//! The field frame through [`Part::EditBorder`]: the part's 2 px border drawn around the field's
//! rect over the caller's fill. The Normal frame's ring is clear, so the caller's `fill` runs right
//! up to the line.

use crate::classic::widgets::FIELD_PAD;
use crate::look::Paint;
use crate::look::widgets::ComboLook;
use crate::pointer::Interaction;
use crate::raster::{Canvas, Rect};
use crate::style::{Part, State, Style};
use crate::theme::Color;

/// [`crate::look::widgets::draw_field_frame`] through `st`: `fill` in the 2 px inset, then the
/// Normal list border over it; `false` when `st` has no edit border part.
pub fn draw_field_frame(st: &Style, c: &mut Canvas, p: &Paint, r: Rect, fill: Color) -> bool {
    draw_frame(st, c, p, r, fill, list_part(st), State::Normal)
}

/// [`crate::look::widgets::draw_edit_frame`] through `st`: as [`draw_field_frame`], with the edit
/// border.
pub fn draw_edit_frame(st: &Style, c: &mut Canvas, p: &Paint, r: Rect, fill: Color) -> bool {
    draw_frame(st, c, p, r, fill, Part::EditBorder, State::Normal)
}

/// The part round a list or a combo box: [`Part::ListBorder`] when `st` has one,
/// [`Part::EditBorder`] otherwise.
fn list_part(st: &Style) -> Part {
    if st.part(Part::ListBorder).is_some() {
        Part::ListBorder
    } else {
        Part::EditBorder
    }
}

/// [`crate::look::widgets::draw_combo_frame`] through `st`. Greyed, the Disabled border goes round
/// `r` and again round the text part left of `drop`, so its ring closes a column short of the
/// button. That greyed ring is 1 px, a drop-down list's; no editable combo is greyed today.
pub fn draw_combo_frame(
    st: &Style,
    c: &mut Canvas,
    p: &Paint,
    r: Rect,
    drop: Rect,
    fill: Color,
    look: ComboLook,
) -> bool {
    let ComboLook {
        enabled,
        focused,
        ptr,
        editable,
    } = look;
    if !editable && st.part(Part::ComboList).is_some() {
        p.fill_back(c, r);
        st.draw(c, r, Part::ComboList, super::state(enabled, ptr), p.s);
        return true;
    }
    if st.part(Part::EditBorder).is_none() {
        return false;
    }
    let part = if editable && st.combo_edit_border {
        Part::EditBorder
    } else {
        list_part(st)
    };
    if enabled {
        draw_well(c, p, drop);
        draw_frame(
            st,
            c,
            p,
            r,
            fill,
            part,
            super::field_state(true, focused, ptr),
        );
        return true;
    }
    draw_frame(st, c, p, r, fill, part, State::Disabled);
    let text = Rect::new(r.x, r.y, drop.x - r.x, r.height);
    st.draw(c, text, part, State::Disabled, p.s);
    let back = text.inset(2 * p.s);
    let s = p.s;
    for side in [
        Rect::new(back.x, back.y, back.width, s),
        Rect::new(back.x, back.bottom() - s, back.width, s),
        Rect::new(back.x, back.y + s, s, back.height - 2 * s),
        Rect::new(back.right() - s, back.y + s, s, back.height - 2 * s),
    ] {
        p.fill_back(c, side);
    }
    draw_well(c, p, drop);
    true
}

fn draw_well(c: &mut Canvas, p: &Paint, r: Rect) {
    c.ring_t(r.inset(-p.s), p.cs.window, p.cs.window, p.s);
}

/// [`crate::look::widgets::draw_spin_field`] through `st`: enabled, a `window` field inside the
/// Normal border.
pub fn draw_spin_field(
    st: &Style,
    c: &mut Canvas,
    p: &Paint,
    r: Rect,
    up: Rect,
    down: Rect,
    enabled: bool,
) -> bool {
    if st.part(Part::EditBorder).is_none() {
        return false;
    }
    let greyed_frame = !enabled && st.greyed_field_frame;
    if enabled || greyed_frame {
        c.fill_rect(r, p.cs.window);
    }
    let state = if greyed_frame {
        State::Disabled
    } else {
        State::Normal
    };
    st.draw(c, r, Part::EditBorder, state, p.s);
    draw_well(c, p, up.union(&down));
    let gap = Rect::new(
        up.x - p.s,
        up.bottom(),
        up.width + 2 * p.s,
        down.y - up.bottom(),
    );
    p.fill_back(c, gap);
    true
}

fn draw_frame(
    st: &Style,
    c: &mut Canvas,
    p: &Paint,
    r: Rect,
    fill: Color,
    part: Part,
    state: State,
) -> bool {
    if st.part(Part::EditBorder).is_none() {
        return false;
    }
    c.ring_t(r, fill, fill, 2 * p.s);
    st.draw(c, r, part, state, p.s);
    true
}

/// [`crate::look::widgets::draw_edit_field`] through `st`: `fill` over the whole rect, the line in
/// the frame [`super::field_state`] picks for an enabled field, and the inner rect the text goes in
/// (`FIELD_PAD` inside, as in Classic); `None` when `st` has no edit border part.
pub fn draw_sunken_field(
    st: &Style,
    c: &mut Canvas,
    p: &Paint,
    r: Rect,
    fill: Color,
    focused: bool,
    ptr: Interaction,
) -> Option<Rect> {
    st.part(Part::EditBorder)?;
    c.fill_rect(r, fill);
    st.draw(
        c,
        r,
        Part::EditBorder,
        super::field_state(true, focused, ptr),
        p.s,
    );
    Some(r.inset(FIELD_PAD * p.s))
}

/// [`crate::look::widgets::draw_disabled_field`] through `st`: what sits behind the field over the
/// whole rect, then the Normal line; in a style with [`Style::greyed_field_frame`], `window` then
/// the Disabled line.
pub fn draw_disabled_field(st: &Style, c: &mut Canvas, p: &Paint, r: Rect) -> Option<Rect> {
    st.part(Part::EditBorder)?;
    if st.greyed_field_frame {
        c.fill_rect(r, p.cs.window);
        st.draw(c, r, Part::EditBorder, State::Disabled, p.s);
    } else {
        p.fill_back(c, r);
        st.draw(c, r, Part::EditBorder, State::Normal, p.s);
    }
    Some(r.inset(FIELD_PAD * p.s))
}

#[cfg(test)]
mod tests {
    use crate::classic;
    use crate::classic::widgets::FIELD_PAD;
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::look::widgets::{draw_field_frame, draw_sunken_field};
    use crate::raster::{Canvas, Rect};
    use crate::theme::Color;

    fn line() -> u32 {
        crate::blue::Variant::Blue
            .scheme()
            .edit_border
            .normal
            .rim
            .argb()
    }

    #[test]
    fn a_classic_look_paints_the_classic_field_frame() {
        let theme = theme_for(Interface::Classic);
        let p = theme.paint();
        let r = Rect::new(2, 2, 40, 20);
        let (mut a, mut b) = (Canvas::new(46, 26), Canvas::new(46, 26));
        draw_field_frame(&mut a, &p, r, p.cs.button_face);
        classic::widgets::draw_field_frame(&mut b, &p, r, p.cs.button_face);
        assert!(a == b, "a Classic look draws Classic's 2 px sunken edge");
        let (mut a, mut b) = (Canvas::new(46, 26), Canvas::new(46, 26));
        let ia = draw_sunken_field(&mut a, &p, r, p.cs.button_face);
        let ib = classic::widgets::draw_sunken_field(&mut b, &p, r, p.cs.button_face);
        assert!(
            a == b && ia == ib,
            "a Classic look draws Classic's sunken field"
        );
    }

    fn blue_frame(under: Color, fill: Color) -> (Canvas, Rect) {
        let theme = theme_for(Interface::Blue);
        let r = Rect::new(2, 2, 40, 20);
        let mut c = Canvas::new(44, 24);
        c.fill_rect(c.bounds(), under);
        draw_field_frame(&mut c, &theme.paint(), r, fill);
        (c, r)
    }

    #[test]
    fn a_blue_field_frame_is_a_1px_line_with_square_corners() {
        let theme = theme_for(Interface::Blue);
        let cs = &theme.colors;
        let (c, r) = blue_frame(cs.button_face, cs.window);
        let (x0, y0, x1, y1) = (r.x, r.y, r.right() - 1, r.bottom() - 1);
        for (x, y) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
            assert_eq!(c.get(x, y), line(), "corner ({x}, {y}) is the line");
        }
        for (x, y) in [(20, y0), (20, y1), (x0, 12), (x1, 12)] {
            assert_eq!(c.get(x, y), line(), "edge ({x}, {y}) is the line");
        }
        assert_eq!(
            c.get(x0 - 1, y0 - 1),
            cs.button_face.argb(),
            "nothing is drawn outside the rect"
        );
    }

    #[test]
    fn a_blue_field_frame_fills_the_band_inside_its_line() {
        let theme = theme_for(Interface::Blue);
        let cs = &theme.colors;
        let (c, r) = blue_frame(cs.button_face, cs.window);
        let (x0, y0, x1, y1) = (r.x, r.y, r.right() - 1, r.bottom() - 1);
        for (x, y) in [
            (x0 + 1, y0 + 1),
            (x1 - 1, y1 - 1),
            (20, y0 + 1),
            (x0 + 1, 12),
        ] {
            assert_eq!(c.get(x, y), cs.window.argb(), "band ({x}, {y}) is window");
        }
        assert_eq!(
            c.get(x0 + 2, y0 + 2),
            cs.button_face.argb(),
            "inside the 2 px inset stays the caller's"
        );
    }

    #[test]
    fn a_disabled_blue_field_keeps_its_line_and_fills_the_band_with_button_face() {
        let theme = theme_for(Interface::Blue);
        let cs = &theme.colors;
        let (c, r) = blue_frame(cs.window, cs.button_face);
        assert_eq!(c.get(r.x, r.y), line(), "the corner stays the line");
        assert_eq!(c.get(20, r.y), line(), "the top edge stays the line");
        assert_eq!(
            c.get(r.x + 1, r.y + 1),
            cs.button_face.argb(),
            "the band is button_face"
        );
    }

    #[test]
    fn a_blue_sunken_field_is_filled_to_its_line_with_text_3px_in() {
        let theme = theme_for(Interface::Blue);
        let p = theme.paint();
        let r = Rect::new(2, 2, 40, 20);
        for fill in [p.cs.window, p.cs.button_face] {
            let mut c = Canvas::new(44, 24);
            let inner = draw_sunken_field(&mut c, &p, r, fill);
            assert_eq!(inner, r.inset(FIELD_PAD), "the text goes 3 px inside");
            assert_eq!(c.get(r.x, r.y), line(), "the corner is the line");
            assert_eq!(
                c.get(r.right() - 1, 12),
                line(),
                "the right edge is the line"
            );
            for (x, y) in [(r.x + 1, r.y + 1), (20, 12)] {
                assert_eq!(c.get(x, y), fill.argb(), "({x}, {y}) is the fill");
            }
        }
    }

    #[test]
    fn blue_fields_at_paint_scale_2_double_every_pixel() {
        use crate::look::widgets::ComboLook;
        use crate::pointer::Interaction;
        let draw = |c: &mut Canvas, p: &crate::look::Paint| {
            let s = p.s;
            let white = p.cs.window;
            crate::look::widgets::draw_field_frame(c, p, Rect::new(2, 2, 60, 21).scaled(s), white);
            crate::look::widgets::draw_edit_field(
                c,
                p,
                Rect::new(2, 26, 60, 21).scaled(s),
                white,
                true,
                Interaction::Idle,
            );
            crate::look::widgets::draw_disabled_field(c, p, Rect::new(2, 50, 60, 21).scaled(s));
            for (y, enabled, editable) in [(74, true, true), (98, false, false), (122, true, false)]
            {
                let r = Rect::new(2, y, 80, 21).scaled(s);
                let drop = Rect::new(64, y + 2, 16, 17).scaled(s);
                let look = ComboLook {
                    enabled,
                    focused: false,
                    ptr: Interaction::Idle,
                    editable,
                };
                crate::look::widgets::draw_combo_frame(c, p, r, drop, white, look);
            }
            let r = Rect::new(66, 2, 40, 21).scaled(s);
            let up = Rect::new(89, 4, 15, 8).scaled(s);
            let down = Rect::new(89, 13, 15, 8).scaled(s);
            crate::look::widgets::draw_spin_field(c, p, r, up, down, true);
        };
        assert!(
            crate::look::styled::fixture::blue_doubles_at_paint_scale_2(110, 145, draw),
            "field, edit, greyed field, combo and spinner frames at s = 2 are their s = 1 pixels doubled"
        );
    }
}
