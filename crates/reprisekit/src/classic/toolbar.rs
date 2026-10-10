//! Classic's toolbar painters: a button's thin bevel, the separator's
//! etched line and a label greyed flat. The toolbar control reaches them
//! through [`crate::look::widgets::toolbar`].

use crate::paint::Paint;
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::style::State;
use crate::text::{Align, TextRenderer};

/// A big separator's line sits this far into its slot and from the band's top and bottom.
pub const BIG_SEP_INSET: i32 = 2;

/// Bevel for `state`: none at rest or greyed, raised when hot, sunken when pressed or latched.
pub fn draw_button(c: &mut Canvas, p: &Paint, r: Rect, state: State) {
    let edge = match state {
        State::Pressed | State::Checked | State::CheckedHot => EdgeStyle::ThinSunken,
        State::Hot => EdgeStyle::ThinRaised,
        _ => return,
    };
    c.edge_t(r, edge, p.cs, p.s);
}

/// Etched line in slot `r`: big strip uses `BIG_SEP_INSET`, small strip centres it.
pub fn draw_separator(c: &mut Canvas, p: &Paint, big: bool, r: Rect) {
    let s = p.s;
    if big {
        let x = r.x + BIG_SEP_INSET * s;
        c.etched_vline_t(
            x,
            r.y + BIG_SEP_INSET * s,
            r.bottom() - BIG_SEP_INSET * s,
            p.cs,
            s,
        );
    } else {
        let x = r.x + r.width / 2 - s;
        c.etched_vline_t(x, r.y + 3 * s, r.bottom() - 3 * s, p.cs, s);
    }
}

/// Label left-aligned in `r`, greyed flat when the button is greyed.
pub fn draw_label(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    r: Rect,
    label: &str,
    state: State,
    px: u32,
) {
    let ink = if state == State::Disabled {
        p.cs.gray_text
    } else {
        p.cs.button_text
    };
    text.draw(c, r, label, false, ink, Align::Left, px);
}
