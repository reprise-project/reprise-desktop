//! The forwarders the task pane paints through: the style's
//! [`Part::TaskPane`], [`Part::TaskGroupHeader`], [`Part::TaskGroupBody`],
//! [`Part::TaskCollapse`], [`Part::TaskExpand`] when it has them, Classic's
//! drawing otherwise. Every rect is in device pixels.

use crate::look::Paint;
use crate::look::styled;
use crate::metrics::TaskMetrics;
use crate::pointer::Interaction;
use crate::raster::{Canvas, Rect};
use crate::style::{Part, State};
use crate::text::{Align, TextRenderer};
use crate::theme::Color;

/// Classic's chevron: its box's left edge this far in from the title
/// band's right edge, and its top this far down the band, open (pointing
/// up) and folded.
const CHEVRON_RIGHT: i32 = 18;
const CHEVRON_Y_OPEN: i32 = 7;
const CHEVRON_Y_SHUT: i32 = 8;
/// One arrow of Classic's chevron, top row first: the columns of the up
/// arrow, the down arrow being the same rows in reverse.
const CHEVRON: [&[i32]; 4] = [&[3], &[2, 3, 4], &[1, 2, 4, 5], &[0, 1, 5, 6]];

/// The pane's background over `r`: the style's [`Part::TaskPane`] when it
/// has one, the window colour otherwise.
pub fn draw_back(c: &mut Canvas, p: &Paint, r: Rect) {
    if let Some(st) = p.look.style()
        && styled::taskpane::draw_back(st, c, r, p.s)
    {
        return;
    }
    c.fill_rect(r, p.cs.window);
}

/// A group's title band `r` with its bold title in `title_r`, and the
/// button that folds it (`open`) or unfolds it. `ptr` is what the pointer
/// does to the band. The style's title band and buttons when it has them,
/// the title in its [`Part::TaskGroupHeader`] ink for the state; Classic's
/// face band with a black double chevron otherwise, the same whatever the
/// pointer does.
#[expect(clippy::too_many_arguments)]
pub fn draw_header(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    r: Rect,
    title_r: Rect,
    title: &str,
    open: bool,
    ptr: Interaction,
    px: u32,
) {
    if let Some(st) = p.look.style()
        && styled::taskpane::draw_header(st, c, text, p, r, title_r, title, open, ptr, px)
    {
        return;
    }
    let Paint { cs, s, .. } = *p;
    c.fill_rect(r, cs.button_face);
    text.draw(c, title_r, title, true, cs.button_text, Align::Left, px);
    let dy = if open { CHEVRON_Y_OPEN } else { CHEVRON_Y_SHUT };
    // A band grown with the text keeps the chevron centred down it.
    let grown = (r.height - TaskMetrics::CLASSIC.header * s).max(0) / 2;
    let cy = r.y + dy * s + grown;
    let cx = r.right() - CHEVRON_RIGHT * s;
    for k in 0..2 {
        draw_chevron(
            c,
            cx,
            cy + k * CHEVRON.len() as i32 * s,
            open,
            cs.button_text,
            s,
        );
    }
}

fn draw_chevron(c: &mut Canvas, x: i32, y: i32, up: bool, ink: Color, s: i32) {
    for i in 0..CHEVRON.len() {
        let row = if up { i } else { CHEVRON.len() - 1 - i };
        for dx in CHEVRON[row] {
            c.fill_rect(Rect::new(x + dx * s, y + i as i32 * s, s, s), ink);
        }
    }
}

/// A group's body `r` under its title band: the style's
/// [`Part::TaskGroupBody`] when it has one; otherwise the window colour
/// inside a 1 px face border down both sides and along the bottom (the
/// title band covers where its top would be).
pub fn draw_body(c: &mut Canvas, p: &Paint, r: Rect) {
    if let Some(st) = p.look.style()
        && styled::taskpane::draw_body(st, c, r, p.s)
    {
        return;
    }
    let Paint { cs, s, .. } = *p;
    c.fill_rect(r, cs.window);
    c.fill_rect(Rect::new(r.x, r.y, s, r.height), cs.button_face);
    c.fill_rect(Rect::new(r.right() - s, r.y, s, r.height), cs.button_face);
    c.fill_rect(Rect::new(r.x, r.bottom() - s, r.width, s), cs.button_face);
}

/// A link's ink: the style's [`Part::TaskGroupBody`] ink, Hot under the
/// pointer, when it has one, the window text otherwise; a dead link's is
/// the scheme's grey text in every look.
pub fn link_ink(p: &Paint, enabled: bool, hot: bool) -> Color {
    if !enabled {
        return p.cs.gray_text;
    }
    let state = if hot { State::Hot } else { State::Normal };
    p.look
        .style()
        .and_then(|st| st.text_color(Part::TaskGroupBody, state))
        .unwrap_or(p.cs.window_text)
}
