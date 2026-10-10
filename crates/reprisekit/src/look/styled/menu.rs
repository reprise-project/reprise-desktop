//! Popup menus and menu bars through the style's menu parts. Each drawer returns `false` when the style lacks its part, so the caller keeps Classic's (or the flat) look. Rects are device px, already scaled by `s`.

use crate::raster::{Canvas, Rect};
use crate::style::{MenuLook, Part, State, Style};

pub fn draw_lit_row(st: &Style, c: &mut Canvas, r: Rect, greyed: bool, s: i32) -> bool {
    if st.part(Part::MenuItem).is_none() {
        return false;
    }
    let state = if greyed { State::Disabled } else { State::Hot };
    st.draw(c, r, Part::MenuItem, state, s);
    true
}

pub fn draw_check(
    st: &Style,
    m: &MenuLook,
    c: &mut Canvas,
    r: Rect,
    radio: bool,
    lit: bool,
    s: i32,
) -> bool {
    let (mark, at) = if radio {
        (Part::MenuRadioMark, m.radio_at)
    } else {
        (Part::MenuCheckMark, m.check_at)
    };
    let Some(img) = st.part(mark) else {
        return false;
    };
    if st.part(Part::MenuCheck).is_none() {
        return false;
    }
    let state = if lit { State::Hot } else { State::Normal };
    st.draw(c, r, Part::MenuCheck, state, s);
    // The mark's size in design px: its atlas holds it at its own scale.
    let (w, h) = (
        img.atlas.width as i32 / img.scale,
        img.atlas.height as i32 / img.scale,
    );
    let to = Rect::new(r.x + at.0 * s, r.y + at.1 * s, w * s, h * s);
    st.draw(c, to, mark, State::Normal, s);
    true
}

pub fn draw_bar(st: &Style, c: &mut Canvas, r: Rect, s: i32) -> bool {
    if st.part(Part::MenuBar).is_none() {
        return false;
    }
    st.draw(c, r, Part::MenuBar, State::Normal, s);
    true
}

pub fn draw_title(st: &Style, c: &mut Canvas, r: Rect, state: State, s: i32) -> bool {
    if st.part(Part::MenuBarItem).is_none() {
        return false;
    }
    st.draw(c, r, Part::MenuBarItem, state, s);
    true
}

/// Shadow cast by a menu filling `r`, past its right and bottom edges; nothing for a look without one.
pub fn draw_shadow(m: &MenuLook, c: &mut Canvas, r: Rect, s: i32) {
    if let Some(sh) = m.shadow {
        super::combodrop::draw_drop_shadow(c, sh, r, s);
    }
}
