//! Rects are device px, already scaled by `s`.

use crate::raster::{Canvas, Rect};
use crate::style::{Part, State, Style};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mark {
    pub selected: bool,
    pub hot: bool,
    pub lead: bool,
    pub dim: bool,
    pub ring: bool,
}

impl Mark {
    fn states(self) -> &'static [State] {
        match self {
            Mark {
                selected: true,
                hot: true,
                ..
            } => &[State::CheckedHot],
            Mark {
                selected: true,
                dim: true,
                ..
            } => &[State::Inactive],
            Mark {
                selected: true,
                lead: true,
                ..
            } => &[State::CheckedHot],
            Mark { selected: true, .. } => &[State::Checked],
            Mark {
                hot: true,
                ring: true,
                ..
            } => &[State::Hot, State::Focused],
            Mark { hot: true, .. } => &[State::Hot],
            Mark { ring: true, .. } => &[State::Focused],
            _ => &[],
        }
    }
}

pub fn boxes_items(st: &Style) -> bool {
    st.part(Part::ListItem).is_some()
}

pub fn draw_item(st: &Style, c: &mut Canvas, r: Rect, m: Mark, s: i32) -> bool {
    if !boxes_items(st) {
        return false;
    }
    for &state in m.states() {
        st.draw(c, r, Part::ListItem, state, s);
    }
    true
}

pub fn draw_desktop_item(st: &Style, c: &mut Canvas, r: Rect, s: i32) -> bool {
    if st.part(Part::DesktopItem).is_none() {
        return false;
    }
    st.draw(c, r, Part::DesktopItem, State::Checked, s);
    true
}

pub fn draw_expander(
    st: &Style,
    c: &mut Canvas,
    (x, y): (i32, i32),
    expanded: bool,
    hot: bool,
    s: i32,
) -> bool {
    let Some(img) = st.part(Part::TreeExpander) else {
        return false;
    };
    let state = match (expanded, hot) {
        (false, false) => State::Normal,
        (false, true) => State::Hot,
        (true, false) => State::Checked,
        (true, true) => State::CheckedHot,
    };
    let cell = img.frame_nth(state, 0);
    let (w, h) = (cell.width / img.scale, cell.height / img.scale);
    let r = Rect::new(x - 7 * s, y - 8 * s, w * s, h * s);
    st.draw(c, r, Part::TreeExpander, state, s);
    true
}
