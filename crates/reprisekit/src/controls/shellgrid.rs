//! The shell view's icon-mode grid (Icons, Tiles, Thumbnails): the row-major cell flow, group headers and cell drawing. `ShellView::top` is a px offset here, not a row count.

use crate::classic::widgets::draw_focus_ring;
use crate::controls::iconview;
use crate::controls::listview::{GROUP_RULE_Y, GroupRule, draw_group_header};
use crate::controls::shellview::{Entry, Group, Layout, Mode, ShellView, draw_rename};
use crate::look::Paint;
use crate::look::widgets::item;
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use crate::textedit::Caret;
use crate::theme::{ColorScheme, TextUnits, Theme};
use std::borrow::Cow;

/// A thumbnail's frame, border included.
pub const THUMB_FRAME: i32 = 96;
pub const THUMB_CELL: (i32, i32) = (125, 139);
pub(crate) const THUMB_FRAME_X: i32 = 17;
pub(crate) const THUMB_FRAME_Y: i32 = 3;
pub(crate) const THUMB_LABEL_GAP: i32 = 2;
pub(crate) const THUMB_RING: i32 = 3;
/// The label's room under the frame, what a thumbnail cell grows with the text.
const THUMB_LABEL_H: i32 = THUMB_CELL.1 - THUMB_FRAME_Y - THUMB_FRAME - THUMB_LABEL_GAP;
pub const TILE_CELL: (i32, i32) = (218, 56);
pub const TILE_ICON: i32 = 48;
pub(crate) const TILE_ICON_OFF: (i32, i32) = (5, 4);
pub(crate) const TILE_TEXT_X: i32 = 58;
pub(crate) const TILE_TEXT_TOPS: [i32; 3] = [20, 15, 6];

pub(crate) fn tile_text_top(n: usize) -> i32 {
    TILE_TEXT_TOPS[n.clamp(1, TILE_TEXT_TOPS.len()) - 1]
}
pub const ICON_CELL: (i32, i32) = (iconview::CELL, iconview::CELL);
/// Header height; a header that follows a group's cells costs `GROUP_GAP` more.
pub const GROUP_HEADER_H: i32 = 31;
pub(crate) const GROUP_GAP: i32 = 12;
pub(crate) const GROUP_INDENT: i32 = 11;
/// The header's title box, 3 px down; its gradient rule fades to `window` over `GROUP_LINE_W` px.
pub(crate) const GROUP_TITLE_Y: i32 = 3;
pub(crate) const GROUP_LINE_W: i32 = 305;
/// First cell row's inset under the body's top; a group header at the very top takes the body's edge instead.
pub(crate) const GRID_TOP: i32 = 4;
pub(crate) const TILE_TOP: i32 = 2;
pub(crate) const LINE_PITCH: i32 = 13;

/// An icon mode's flow at a text size: the cells' text and the group headers grow by `u`, icons and air stay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Grid {
    pub cell: (i32, i32),
    top: i32,
    header_h: i32,
}

impl Grid {
    /// `mode`'s grid; `List` and `Details` have their own layout and are not asked.
    pub(crate) fn of(mode: Mode, u: TextUnits) -> Grid {
        let (cell, top) = match mode {
            Mode::Thumbnails => (
                (
                    THUMB_CELL.0 + u.len(THUMB_FRAME) - THUMB_FRAME,
                    THUMB_CELL.1 + u.len(THUMB_LABEL_H) - THUMB_LABEL_H,
                ),
                GRID_TOP,
            ),
            Mode::Tiles => (
                (
                    TILE_TEXT_X + u.len(TILE_CELL.0 - TILE_TEXT_X),
                    u.len(TILE_CELL.1),
                ),
                TILE_TOP,
            ),
            _ => {
                let c = iconview::Cell::grown(u);
                ((c.w, c.h), GRID_TOP)
            }
        };
        Grid {
            cell,
            top,
            header_h: u.len(GROUP_HEADER_H),
        }
    }
}

/// The icon modes' row-major flow: one rect per entry on a `cell` grid filling `body`'s width, a full-width header rect before each group's first entry (`GROUP_GAP` of air first, unless it opens the view). A grouped view's cells start `GROUP_INDENT` in. `top` is the px already scrolled away, subtracted from every rect.
pub(crate) fn grid(
    g: &Grid,
    body: Rect,
    count: usize,
    groups: &[Group],
    top: i32,
) -> (Vec<Rect>, Vec<Rect>) {
    let (mut cells, mut headers) = (Vec::with_capacity(count), Vec::new());
    flow(
        g,
        body,
        count,
        groups,
        top,
        |r| cells.push(r),
        |r| headers.push(r),
    );
    (cells, headers)
}

pub(crate) fn grid_height(g: &Grid, body: Rect, count: usize, groups: &[Group]) -> i32 {
    flow(g, body, count, groups, 0, |_| {}, |_| {})
}

/// The flow `grid` and `grid_height` share; `groups` is sorted by `first`, so one cursor walks it.
fn flow(
    g: &Grid,
    body: Rect,
    count: usize,
    groups: &[Group],
    top: i32,
    mut on_cell: impl FnMut(Rect),
    mut on_header: impl FnMut(Rect),
) -> i32 {
    let cell = g.cell;
    let indent = if groups.is_empty() { 0 } else { GROUP_INDENT };
    let cols = ((body.width - indent) / cell.0).max(1);
    let mut y = g.top;
    let mut col = 0;
    let mut next_group = groups.iter().map(|g| g.first).peekable();
    let mut open = false;
    for i in 0..count {
        let mut starts = false;
        while next_group.next_if(|&first| first <= i).is_some() {
            starts = true;
        }
        if starts {
            if open {
                y += cell.1 + GROUP_GAP;
            } else if i == 0 {
                y = 0;
            }
            on_header(Rect::new(body.x, body.y + y - top, body.width, g.header_h));
            y += g.header_h;
            col = 0;
        }
        if col == cols {
            y += cell.1;
            col = 0;
        }
        on_cell(Rect::new(
            body.x + indent + col * cell.0,
            body.y + y - top,
            cell.0,
            cell.1,
        ));
        col += 1;
        open = true;
    }
    y + if open { cell.1 } else { 0 }
}

pub(crate) fn tile_lines(e: &Entry) -> Vec<&str> {
    std::iter::once(e.name.as_str())
        .chain(e.detail.iter().map(String::as_str))
        .collect()
}

pub(crate) fn block_h(n: usize, px: u32, u: TextUnits) -> i32 {
    (n.max(1) as i32 - 1) * u.len(LINE_PITCH) + px as i32
}

pub(crate) fn widths(text: &TextRenderer, lines: &[&str], px: u32) -> (i32, Vec<i32>) {
    let each: Vec<i32> = lines.iter().map(|l| text.measure(l, false, px)).collect();
    (each.iter().copied().max().unwrap_or(0), each)
}

/// A thumbnail cell's frame, centred in the room a grown cell gains.
pub(crate) fn thumb_frame(cell: Rect) -> Rect {
    Rect::new(
        cell.x + THUMB_FRAME_X + (cell.width - THUMB_CELL.0) / 2,
        cell.y + THUMB_FRAME_Y,
        THUMB_FRAME,
        THUMB_FRAME,
    )
}

pub(crate) fn thumb_lines(text: &TextRenderer, name: &str, px: u32, u: TextUnits) -> Vec<String> {
    iconview::wrap_label(text, name, u.len(THUMB_FRAME), px)
}

/// A `Theme` for `iconview::draw_cell` with `desktop_text` swapped for `window_text`: the label sits on the view's `window`.
fn cell_theme(p: &Paint, px: u32) -> Theme {
    let mut theme = Theme::for_draw(p, px);
    theme.colors.desktop_text = p.cs.window_text;
    theme
}

/// The icon `mode` draws for `e`, faded to half (a copy) when the entry is cut. Thumbnails fall back to the 32 px icon.
pub(crate) fn entry_icon(e: &Entry, mode: Mode) -> Option<Cow<'_, Canvas>> {
    let icon = match mode {
        Mode::Tiles => e.icon48.as_deref(),
        Mode::Icons => e.icon32.as_deref(),
        Mode::Thumbnails => e.thumb.as_ref().or(e.icon32.as_deref()),
        Mode::List | Mode::Details => e.icon.as_deref(),
    }?;
    if !e.cut {
        return Some(Cow::Borrowed(icon));
    }
    let mut icon = icon.clone();
    icon.fade(1, 2);
    Some(Cow::Owned(icon))
}

/// One tile: the 48 px `icon`, centred down a grown cell, then the name and `detail` lines in one block, filled with the highlight when selected. `cell` and `label` are already scaled and offset into the canvas.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_tile(
    c: &mut Canvas,
    text: &TextRenderer,
    cs: &ColorScheme,
    e: &Entry,
    icon: Option<&Canvas>,
    cell: Rect,
    label: Rect,
    sel: (bool, bool),
    px: u32,
    s: i32,
    u: TextUnits,
) {
    let (selected, ring) = sel;
    if let Some(icon) = icon {
        let dy = TILE_ICON_OFF.1 + (cell.height / s - TILE_CELL.1) / 2;
        let (ix, iy) = (cell.x + TILE_ICON_OFF.0 * s, cell.y + dy * s);
        c.blit_over(ix, iy, icon);
        if e.link {
            iconview::draw_shortcut_arrow(c, ix, iy + TILE_ICON * s, s);
        }
    }
    if selected {
        c.fill_rect(label, cs.hilight);
    }
    let avail = (cell.width - TILE_TEXT_X * s).max(0);
    for (k, line) in tile_lines(e).iter().enumerate() {
        let fg = match (selected, k) {
            (true, _) => cs.hilight_text,
            (false, 0) => cs.window_text,
            (false, _) => cs.gray_text,
        };
        let r = Rect::new(
            label.x,
            label.y + k as i32 * u.len(LINE_PITCH) * s,
            avail,
            px as i32 + 3 * s,
        );
        let shown = text.ellipsize(line, avail, false, px);
        text.draw(c, r, &shown, false, fg, Align::Left, px);
    }
    if ring {
        let ink = if selected {
            cs.hilight_text
        } else {
            cs.window_text
        };
        draw_focus_ring(c, label.inset(-s), s, ink);
    }
}

/// One thumbnail: the framed image `img` (or the 32 px icon centred in it), a `THUMB_RING` highlight ring when selected, and the wrapped label under it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_thumb(
    c: &mut Canvas,
    text: &TextRenderer,
    cs: &ColorScheme,
    e: &Entry,
    img: Option<&Canvas>,
    cell: Rect,
    label: Rect,
    sel: (bool, bool),
    px: u32,
    s: i32,
    u: TextUnits,
) {
    let (selected, ring) = sel;
    let at = thumb_frame(Rect::new(0, 0, cell.width / s, cell.height / s));
    let frame = Rect::new(
        cell.x + at.x * s,
        cell.y + at.y * s,
        THUMB_FRAME * s,
        THUMB_FRAME * s,
    );
    if selected {
        c.fill_rect(frame.inset(-(THUMB_RING * s)), cs.hilight);
    }
    c.fill_rect(frame, cs.window);
    c.ring_t(frame, cs.button_face, cs.button_face, s);
    if let Some(img) = img {
        let ix = frame.x + (frame.width - img.width as i32) / 2;
        let iy = frame.y + (frame.height - img.height as i32) / 2;
        c.blit_over(ix, iy, img);
        if e.link {
            iconview::draw_shortcut_arrow(c, ix, iy + img.height as i32, s);
        }
    }
    if selected {
        c.fill_rect(label, cs.hilight);
    }
    let fg = if selected {
        cs.hilight_text
    } else {
        cs.window_text
    };
    for (k, line) in thumb_lines(text, &e.name, px, u).iter().enumerate() {
        let r = Rect::new(
            label.x,
            label.y + k as i32 * u.len(LINE_PITCH) * s,
            label.width,
            px as i32 + 3 * s,
        );
        text.draw(c, r, line, false, fg, Align::Center, px);
    }
    if ring {
        draw_focus_ring(c, label.inset(-s), s, fg);
    }
}

/// The three icon modes' body, into a canvas of its own so a cell scrolled past an edge clips.
pub(crate) fn draw_grid(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    sv: &ShellView,
    lay: &Layout,
    px: u32,
) {
    let Paint { cs, s, .. } = *p;
    let u = lay.controls.text;
    let body = lay.body.scaled(s);
    let mut bc = c.scratch(body.width.max(0) as u32, body.height.max(0) as u32);
    let all = Rect::new(0, 0, body.width, body.height);
    bc.fill_rect(all, cs.window);
    // Scaled and moved into the scratch canvas' own space.
    let off = |r: Rect| {
        let r = r.scaled(s);
        Rect::new(r.x - body.x, r.y - body.y, r.width, r.height)
    };
    for (h, g) in lay.headers.iter().zip(&sv.groups) {
        let r = off(*h);
        if !r.intersect(&all).is_empty() {
            let title_box = (u.len(GROUP_TITLE_Y) * s, px as i32 + 3 * s);
            let rule = GroupRule {
                from: cs.gradient_active_title,
                to: cs.window,
                width: GROUP_LINE_W,
                y: u.len(GROUP_RULE_Y),
            };
            draw_group_header(&mut bc, text, cs, r, &g.title, title_box, &rule, px, s);
        }
    }
    let theme = cell_theme(p, px);
    for (i, e) in sv.entries.iter().enumerate() {
        let Some(&at) = lay.cells.get(i) else {
            break;
        };
        if off(at).intersect(&all).is_empty() {
            continue;
        }
        let label = off(label_of(sv.mode, at, e, text, px, u));
        let cell = off(at);
        let editing = matches!(&sv.edit, Some((j, _)) if *j == i);
        // A look that boxes its items boxes the cell, from its top to 2 px under the label (a tile's whole cell), and draws the rest as if nothing were selected.
        let foot = match sv.mode {
            Mode::Tiles => cell.bottom(),
            _ => (label.bottom() + 2 * s).min(cell.bottom()),
        };
        let item_box = Rect::new(cell.x, cell.y, cell.width, foot - cell.y);
        let mark = if editing {
            Default::default()
        } else {
            sv.mark(i)
        };
        let boxed = item::draw_item(&mut bc, p, item_box, mark);
        let selected = !boxed && (sv.selection.is(i) || sv.drop_target == Some(i));
        let ring =
            !boxed && sv.focus_ring && sv.selection.focus == Some(i) && sv.drop_target != Some(i);
        match sv.mode {
            Mode::Icons => {
                let sel = selected.then(|| iconview::Selection {
                    ring,
                    origin: (cell.x / s, cell.y / s),
                });
                let rename = match &sv.edit {
                    Some((j, ed)) if *j == i => Some((ed, Caret::Shown)),
                    _ => None,
                };
                let icon = entry_icon(e, Mode::Icons);
                iconview::draw_cell_at(
                    &mut bc,
                    (cell.x, cell.y),
                    iconview::Cell {
                        w: at.width,
                        h: at.height,
                    },
                    &theme,
                    text,
                    &e.name,
                    icon.as_deref(),
                    e.link,
                    sel,
                    rename,
                );
            }
            Mode::Tiles => {
                draw_tile(
                    &mut bc,
                    text,
                    cs,
                    e,
                    entry_icon(e, Mode::Tiles).as_deref(),
                    cell,
                    label,
                    (selected, ring),
                    px,
                    s,
                    u,
                );
                if editing {
                    draw_rename(&mut bc, text, p, sv, i, label, px);
                }
            }
            _ => {
                draw_thumb(
                    &mut bc,
                    text,
                    cs,
                    e,
                    entry_icon(e, Mode::Thumbnails).as_deref(),
                    cell,
                    label,
                    (selected, ring),
                    px,
                    s,
                    u,
                );
                if editing {
                    draw_rename(&mut bc, text, p, sv, i, label, px);
                }
            }
        }
    }
    if let Some(m) = sv.marquee {
        let m = off(m).intersect(&all);
        if !m.is_empty() {
            crate::look::widgets::draw_marquee(&mut bc, p, m, cs.window_text);
        }
    }
    c.blit_canvas(body.x, body.y, &bc);
}

/// The label rect for an icon-mode cell at text size `u`: what `draw` highlights and where a rename box goes.
pub(crate) fn label_of(
    mode: Mode,
    cell: Rect,
    e: &Entry,
    text: &TextRenderer,
    px: u32,
    u: TextUnits,
) -> Rect {
    match mode {
        Mode::Icons => {
            let r = iconview::label_layout(text, &e.name, px, 1, cell.width).rect;
            Rect::new(cell.x + r.x, cell.y + r.y, r.width, r.height)
        }
        Mode::Tiles => {
            let lines = tile_lines(e);
            let avail = (cell.width - TILE_TEXT_X).max(0);
            let shown: Vec<String> = lines
                .iter()
                .map(|l| text.ellipsize(l, avail, false, px).into_owned())
                .collect();
            let (widest, _) = widths(
                text,
                &shown.iter().map(String::as_str).collect::<Vec<_>>(),
                px,
            );
            let h = block_h(shown.len(), px, u);
            Rect::new(
                cell.x + TILE_TEXT_X,
                cell.y + u.len(tile_text_top(shown.len())),
                widest,
                h,
            )
        }
        _ => {
            let frame = thumb_frame(cell);
            let lines = thumb_lines(text, &e.name, px, u);
            let (widest, _) = widths(
                text,
                &lines.iter().map(String::as_str).collect::<Vec<_>>(),
                px,
            );
            let h = block_h(lines.len(), px, u);
            Rect::new(
                frame.x + (frame.width - widest) / 2,
                frame.bottom() + THUMB_LABEL_GAP,
                widest,
                h,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classic::widgets::FIELD_PAD;
    use crate::controls::metrics::ControlMetrics;
    use crate::controls::shellview::layout;
    use crate::controls::shellview::tests::{iconed, rend, render, square};
    use crate::textedit::TextEdit;
    use crate::theme::Color;

    const EXPECTED_THUMB_FRAME: i32 = 96;
    const EXPECTED_THUMB_PITCH: (i32, i32) = (125, 139);
    const EXPECTED_THUMB_OFF: (i32, i32) = (17, 3);
    const EXPECTED_THUMB_RING: i32 = 3;
    const EXPECTED_TILE_PITCH: (i32, i32) = (218, 56);
    const EXPECTED_TILE_TEXT_X: i32 = 58;
    const EXPECTED_TILE_ICON_OFF: (i32, i32) = (5, 4);
    const EXPECTED_TILE_TEXT_TOP: i32 = 20;
    const EXPECTED_ICON_PITCH: i32 = 75;
    const EXPECTED_GROUP_PITCH: i32 = 99;
    const EXPECTED_GROUP_RULE_Y: i32 = 18;
    const EXPECTED_GROUP_INDENT: i32 = 11;

    #[test]
    fn thumbnails_flow_at_their_pitch() {
        let sv = iconed(6, Mode::Thumbnails);
        // Three columns wide (3 * 125 plus the sunken inset), two rows.
        let r = Rect::new(0, 0, 3 * EXPECTED_THUMB_PITCH.0 + 2 * FIELD_PAD, 400);
        let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
        assert_eq!(lay.cells[0].width, EXPECTED_THUMB_PITCH.0);
        assert_eq!(lay.cells[0].height, EXPECTED_THUMB_PITCH.1);
        assert_eq!(lay.cells[1].x, lay.cells[0].x + EXPECTED_THUMB_PITCH.0);
        assert_eq!(lay.cells[1].y, lay.cells[0].y, "row-major, not column");
        assert_eq!(
            lay.cells[3].y,
            lay.cells[0].y + EXPECTED_THUMB_PITCH.1,
            "the fourth wraps to the second row"
        );
        assert_eq!(lay.cells[3].x, lay.cells[0].x);
        let frame = thumb_frame(lay.cells[0]);
        assert_eq!(
            (frame.x - lay.cells[0].x, frame.y - lay.cells[0].y),
            EXPECTED_THUMB_OFF
        );
        assert_eq!(
            (frame.width, frame.height),
            (EXPECTED_THUMB_FRAME, EXPECTED_THUMB_FRAME)
        );
    }

    #[test]
    fn tiles_wrap_row_major_at_the_body_width() {
        let (text, px) = rend();
        let sv = iconed(5, Mode::Tiles);
        let r = Rect::new(0, 0, 2 * EXPECTED_TILE_PITCH.0 + 2 * FIELD_PAD, 300);
        let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
        assert_eq!(
            (lay.cells[0].width, lay.cells[0].height),
            (EXPECTED_TILE_PITCH.0, EXPECTED_TILE_PITCH.1)
        );
        assert_eq!(lay.cells[1].x, lay.cells[0].x + EXPECTED_TILE_PITCH.0);
        assert_eq!(lay.cells[2].y, lay.cells[0].y + EXPECTED_TILE_PITCH.1);
        assert_eq!(lay.cells[2].x, lay.cells[0].x, "two columns, then wrap");
        let label = crate::controls::shellview::label_rect(&sv, &lay, 0, &text, px);
        assert_eq!(label.x, lay.cells[0].x + EXPECTED_TILE_TEXT_X);
        assert_eq!(label.y, lay.cells[0].y + EXPECTED_TILE_TEXT_TOP);
    }

    #[test]
    fn a_tiles_text_block_rises_with_its_lines() {
        const EXPECTED_TOPS: [i32; 3] = [20, 15, 6];
        let (text, px) = rend();
        let mut sv = iconed(3, Mode::Tiles);
        sv.entries[1].detail = vec!["1 KB".into()];
        sv.entries[2].detail = vec!["Text Document".into(), "1 KB".into()];
        let r = Rect::new(0, 0, 3 * EXPECTED_TILE_PITCH.0 + 2 * FIELD_PAD, 300);
        let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
        for (i, top) in EXPECTED_TOPS.into_iter().enumerate() {
            let label = crate::controls::shellview::label_rect(&sv, &lay, i, &text, px);
            assert_eq!(label.y - lay.cells[i].y, top, "{} line(s)", i + 1);
        }
    }

    #[test]
    fn icons_flow_row_major_on_iconviews_cell() {
        let sv = iconed(4, Mode::Icons);
        let r = Rect::new(0, 0, 2 * EXPECTED_ICON_PITCH + 2 * FIELD_PAD, 300);
        let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
        assert_eq!(lay.cells[0].width, EXPECTED_ICON_PITCH);
        assert_eq!(lay.cells[1].x, lay.cells[0].x + EXPECTED_ICON_PITCH);
        assert_eq!(lay.cells[1].y, lay.cells[0].y);
        assert_eq!(lay.cells[2].y, lay.cells[0].y + EXPECTED_ICON_PITCH);
    }

    #[test]
    fn a_group_inserts_a_header_and_starts_a_new_row() {
        let mut sv = iconed(4, Mode::Tiles);
        sv.groups = vec![
            Group {
                title: "Folders".into(),
                first: 0,
            },
            Group {
                title: "Files".into(),
                first: 1,
            },
        ];
        let r = Rect::new(0, 0, 2 * EXPECTED_TILE_PITCH.0 + 2 * FIELD_PAD, 400);
        let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
        assert_eq!(lay.headers.len(), 2);
        assert_eq!(lay.headers[0].width, lay.body.width, "full body width");
        assert_eq!(lay.headers[0].height, GROUP_HEADER_H);
        assert_eq!(lay.headers[0].y, lay.body.y, "the first one opens the body");
        assert_eq!(lay.cells[0].y, lay.headers[0].bottom());
        assert_eq!(
            lay.headers[1].y - lay.headers[0].y,
            EXPECTED_GROUP_PITCH,
            "one tile row plus a header"
        );
        assert_eq!(lay.cells[1].y, lay.headers[1].bottom());
        assert_eq!(
            lay.cells[1].x,
            lay.body.x + EXPECTED_GROUP_INDENT,
            "a group starts a new row, indented"
        );
        assert_eq!(lay.headers[1].x, lay.body.x, "the header keeps the edge");
    }

    #[test]
    fn a_group_header_draws_its_rule_18_px_down() {
        let mut sv = iconed(2, Mode::Tiles);
        sv.groups = vec![Group {
            title: "Folders".into(),
            first: 0,
        }];
        let (c, lay, theme) = render(&sv, Rect::new(0, 0, 500, 300));
        let cs = &theme.colors;
        let h = lay.headers[0];
        assert_eq!(
            c.get(h.x, h.y + EXPECTED_GROUP_RULE_Y),
            cs.gradient_active_title.argb()
        );
        assert_eq!(
            c.get(h.x, h.y + EXPECTED_GROUP_RULE_Y + 1),
            cs.window.argb(),
            "one row, not a band"
        );
        assert_ne!(
            c.get(h.x + 200, h.y + EXPECTED_GROUP_RULE_Y),
            cs.gradient_active_title.argb(),
            "it fades towards the window colour"
        );
    }

    #[test]
    fn content_taller_than_the_body_gets_a_vertical_scrollbar() {
        let short = layout(
            &iconed(2, Mode::Icons),
            Rect::new(0, 0, 400, 200),
            &ControlMetrics::CLASSIC,
        );
        assert!(short.vscroll.is_none(), "two rows of icons fit");
        let full = layout(
            &iconed(60, Mode::Icons),
            Rect::new(0, 0, 400, 200),
            &ControlMetrics::CLASSIC,
        );
        assert!(full.vscroll.is_some());
        assert!(
            full.hscroll.is_none(),
            "the icon modes never scroll sideways"
        );
        assert!(full.content.1 > full.body.height);
    }

    #[test]
    fn a_selected_thumbnail_is_ringed_with_the_hilight() {
        let mut sv = iconed(2, Mode::Thumbnails);
        for e in &mut sv.entries {
            e.thumb = Some(square(80, Color::rgb(0, 128, 0)));
        }
        sv.selection.set_only(Some(0));
        let (c, lay, theme) = render(&sv, Rect::new(0, 0, 400, 300));
        let cs = &theme.colors;
        let frame = thumb_frame(lay.cells[0]);
        let mid = frame.y + frame.height / 2;
        for d in 1..=EXPECTED_THUMB_RING {
            assert_eq!(
                c.get(frame.x - d, mid),
                cs.hilight.argb(),
                "{d} px left of the frame is on the ring"
            );
        }
        assert_eq!(
            c.get(frame.x - EXPECTED_THUMB_RING - 1, mid),
            cs.window.argb(),
            "and the fourth is not"
        );
        assert_eq!(
            c.get(frame.x + frame.width / 2, frame.y - 1),
            cs.hilight.argb()
        );
        assert_eq!(
            c.get(frame.x + frame.width / 2, frame.bottom()),
            cs.hilight.argb()
        );
        let other = thumb_frame(lay.cells[1]);
        assert_eq!(c.get(other.x - 1, other.y + 10), cs.window.argb());
    }

    #[test]
    fn a_thumbnail_without_an_image_centres_the_icon() {
        let mut sv = iconed(1, Mode::Thumbnails);
        sv.entries[0].thumb = None;
        let (c, lay, theme) = render(&sv, Rect::new(0, 0, 300, 300));
        let cs = &theme.colors;
        let frame = thumb_frame(lay.cells[0]);
        assert_eq!(c.get(frame.x, frame.y), cs.button_face.argb(), "the border");
        assert_eq!(c.get(frame.x + 4, frame.y + 4), cs.window.argb());
        let red = Color::rgb(255, 0, 0).argb();
        let (cx, cy) = (frame.x + frame.width / 2, frame.y + frame.height / 2);
        assert_eq!(c.get(cx, cy), red, "the 32 px icon, centred in the frame");
        assert_eq!(
            c.get(cx - iconview::ICON_PX / 2 - 1, cy),
            cs.window.argb(),
            "and no wider than 32 px"
        );
    }

    #[test]
    fn a_marquee_draws_a_dotted_rectangle() {
        let mut sv = iconed(4, Mode::Icons);
        let band = Rect::new(20, 20, 60, 40);
        sv.marquee = Some(band);
        let (c, _, theme) = render(&sv, Rect::new(0, 0, 300, 300));
        let cs = &theme.colors;
        assert_eq!(c.get(band.x, band.y), cs.window_text.argb());
        assert_eq!(
            c.get(band.x + 1, band.y),
            cs.window.argb(),
            "dotted, not solid"
        );
        assert_eq!(c.get(band.x + 2, band.y), cs.window_text.argb());
        assert_eq!(c.get(band.x, band.bottom() - 1), cs.window_text.argb());
    }

    #[test]
    fn a_glass_marquee_is_a_translucent_box() {
        let mut sv = iconed(4, Mode::Icons);
        let band = Rect::new(200, 150, 60, 40);
        sv.marquee = Some(band);
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        let r = Rect::new(0, 0, 300, 300);
        let (text, px) = rend();
        let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
        let mut c = Canvas::new(r.width as u32, r.height as u32);
        crate::controls::shellview::draw(&mut c, &text, &theme.paint(), &sv, &lay, None, px);
        let at = |x, y| c.get(x, y);
        assert_eq!(at(band.x, band.y), 0xFF33_99FF);
        assert_eq!(at(band.x + 1, band.y), 0xFF33_99FF, "solid, not dotted");
        assert_eq!(at(band.right() - 1, band.bottom() - 1), 0xFF33_99FF);
        assert_eq!(
            at(band.x + 30, band.y + 20),
            0xFFB9_D5F1,
            "the fill over white"
        );
    }

    #[test]
    fn a_cut_entry_draws_its_icon_faded() {
        let mut sv = iconed(2, Mode::Tiles);
        sv.entries[1].cut = true;
        let (c, lay, _) = render(&sv, Rect::new(0, 0, 400, 300));
        let at = |cell: Rect| {
            c.get(
                cell.x + EXPECTED_TILE_ICON_OFF.0 + 4,
                cell.y + EXPECTED_TILE_ICON_OFF.1 + 4,
            )
        };
        assert_eq!(at(lay.cells[0]), Color::rgb(255, 0, 0).argb(), "solid red");
        let faded = at(lay.cells[1]);
        assert_ne!(faded, Color::rgb(255, 0, 0).argb());
        assert!(
            faded & 0xFF > 0x40,
            "half the icon's alpha lets the white through: {faded:08x}"
        );
    }

    #[test]
    fn a_shortcut_gets_the_overlay() {
        let mut sv = iconed(2, Mode::Tiles);
        sv.entries[1].link = true;
        let (c, lay, _) = render(&sv, Rect::new(0, 0, 400, 300));
        let black = Color::rgb(0, 0, 0).argb();
        let bottom = |cell: Rect| {
            c.get(
                cell.x + EXPECTED_TILE_ICON_OFF.0,
                cell.y + EXPECTED_TILE_ICON_OFF.1 + TILE_ICON - 1,
            )
        };
        assert_eq!(
            bottom(lay.cells[1]),
            black,
            "the overlay's own bottom row is black"
        );
        assert_ne!(bottom(lay.cells[0]), black);
    }

    /// `sv` laid out and drawn in Classic at `pct` text size, with the size's controls and font.
    fn render_at(sv: &ShellView, r: Rect, pct: u32) -> (Canvas, Layout, Theme) {
        let theme = Theme::default().with_text_size(pct);
        let text = TextRenderer::load(&theme.fonts);
        let lay = layout(sv, r, &theme.controls());
        let mut c = Canvas::new(r.width as u32, r.height as u32);
        let px = theme.fonts.size_px;
        crate::controls::shellview::draw(&mut c, &text, &theme.paint(), sv, &lay, None, px);
        (c, lay, theme)
    }

    #[test]
    fn the_icon_modes_cells_grow_with_the_text_size() {
        const EXPECTED: [(Mode, [(i32, i32); 3]); 3] = [
            (Mode::Icons, [(75, 75), (116, 95), (150, 112)]),
            (Mode::Tiles, [(218, 56), (305, 87), (378, 112)]),
            (Mode::Thumbnails, [(125, 139), (177, 160), (221, 177)]),
        ];
        let r = Rect::new(0, 0, 1000, 800);
        for (mode, sizes) in EXPECTED {
            for (pct, size) in [100, 150, 200].into_iter().zip(sizes) {
                let sv = iconed(4, mode);
                let cm = Theme::default().with_text_size(pct).controls();
                let lay = layout(&sv, r, &cm);
                let c = lay.cells[0];
                assert_eq!((c.width, c.height), size, "{mode:?} at {pct}%");
                assert_eq!(
                    lay.cells[1].x - c.x,
                    size.0,
                    "{mode:?} at {pct}%: the pitch"
                );
            }
        }
    }

    /// The icons keep their size in a grown cell: a tile's 48 px icon stays 5 px in and centres down the cell.
    #[test]
    fn a_grown_tile_keeps_its_icon_and_spaces_its_lines_at_the_grown_pitch() {
        // (pct, icon top, text top, block height): three lines at a 20 px pitch at 150%, 26 px at 200%.
        const EXPECTED: [(u32, i32, i32, i32); 2] =
            [(150, 19, 9, 2 * 20 + 17), (200, 32, 12, 2 * 26 + 22)];
        let mut sv = iconed(2, Mode::Tiles);
        sv.entries[0].detail = vec!["Text Document".into(), "1 KB".into()];
        sv.selection.set_only(Some(0));
        let red = Color::rgb(255, 0, 0).argb();
        for (pct, icon_top, top, h) in EXPECTED {
            let (c, lay, theme) = render_at(&sv, Rect::new(0, 0, 900, 300), pct);
            let text = TextRenderer::load(&theme.fonts);
            let cell = lay.cells[0];
            let (ix, iy) = (cell.x + EXPECTED_TILE_ICON_OFF.0, cell.y + icon_top);
            assert_eq!(c.get(ix, iy), red, "{pct}%: the icon's top-left");
            assert_eq!(c.get(ix + TILE_ICON - 1, iy + TILE_ICON - 1), red, "{pct}%");
            assert_ne!(c.get(ix, iy - 1), red, "{pct}%: 48 px, not grown");
            assert_ne!(c.get(ix + TILE_ICON, iy), red, "{pct}%");
            let label =
                crate::controls::shellview::label_rect(&sv, &lay, 0, &text, theme.fonts.size_px);
            assert_eq!(label.x, cell.x + EXPECTED_TILE_TEXT_X, "{pct}%");
            assert_eq!((label.y - cell.y, label.height), (top, h), "{pct}%");
            let hil = theme.colors.hilight.argb();
            assert_eq!(c.get(label.x, label.y), hil, "{pct}%: the block is filled");
            assert_eq!(c.get(label.x, label.bottom() - 1), hil, "{pct}%");
            assert_ne!(c.get(label.x, label.bottom()), hil, "{pct}%");
        }
    }

    /// A grown thumbnail keeps its 96 px frame, centred in the wider cell, and wraps its label to the grown width.
    #[test]
    fn a_grown_thumbnail_centres_its_frame_over_a_wider_label() {
        const EXPECTED_FRAME_X: [(u32, i32); 3] = [(100, 17), (150, 43), (200, 65)];
        let mut sv = iconed(2, Mode::Thumbnails);
        sv.entries[0].name = "Holiday photos from the hills".into();
        sv.selection.set_only(Some(0));
        for (pct, x) in EXPECTED_FRAME_X {
            let (c, lay, theme) = render_at(&sv, Rect::new(0, 0, 600, 400), pct);
            let text = TextRenderer::load(&theme.fonts);
            let px = theme.fonts.size_px;
            let cell = lay.cells[0];
            let frame = thumb_frame(cell);
            assert_eq!(frame.x - cell.x, x, "{pct}%");
            assert_eq!(
                (frame.width, frame.height),
                (EXPECTED_THUMB_FRAME, EXPECTED_THUMB_FRAME)
            );
            let hil = theme.colors.hilight.argb();
            assert_eq!(
                c.get(frame.x - EXPECTED_THUMB_RING, frame.y + 10),
                hil,
                "{pct}%: the ring"
            );
            assert_eq!(
                c.get(frame.x - EXPECTED_THUMB_RING - 1, frame.y + 10),
                theme.colors.window.argb()
            );
            let u = theme.controls().text;
            let lines = thumb_lines(&text, &sv.entries[0].name, px, u);
            assert_eq!(lines.len(), 2, "{pct}%: {lines:?}");
            let widest = lines
                .iter()
                .map(|l| text.measure(l, false, px))
                .max()
                .unwrap();
            assert!(widest <= u.len(EXPECTED_THUMB_FRAME), "{pct}%: {widest}");
            let label = crate::controls::shellview::label_rect(&sv, &lay, 0, &text, px);
            assert_eq!(label.y, frame.bottom() + THUMB_LABEL_GAP, "{pct}%");
            assert_eq!(
                label.height,
                u.len(LINE_PITCH) + px as i32,
                "{pct}%: two lines"
            );
            assert!((label.x + label.width / 2 - (frame.x + frame.width / 2)).abs() <= 1);
            assert!(
                label.bottom() <= cell.bottom(),
                "{pct}%: the label fits its cell"
            );
        }
    }

    /// An icon cell's label wraps in the grown width and centres under the 32 px icon, which keeps its place.
    #[test]
    fn a_grown_icon_cell_centres_its_label_under_the_icon() {
        let mut sv = iconed(2, Mode::Icons);
        sv.entries[0].name = "Holiday photos from the hills".into();
        for pct in [150, 200] {
            let (c, lay, theme) = render_at(&sv, Rect::new(0, 0, 600, 400), pct);
            let text = TextRenderer::load(&theme.fonts);
            let cell = lay.cells[0];
            let ix = cell.x + (cell.width - iconview::ICON_PX + 1) / 2;
            let red = Color::rgb(255, 0, 0).argb();
            assert_eq!(c.get(ix, cell.y + iconview::ICON_TOP), red, "{pct}%");
            assert_ne!(
                c.get(ix + iconview::ICON_PX, cell.y + iconview::ICON_TOP),
                red
            );
            let label =
                crate::controls::shellview::label_rect(&sv, &lay, 0, &text, theme.fonts.size_px);
            assert_eq!(
                label.y - cell.y,
                iconview::ICON_TOP + iconview::ICON_PX + 4,
                "{pct}%"
            );
            assert!(
                (label.x + label.width / 2 - (cell.x + cell.width / 2)).abs() <= 1,
                "{pct}%"
            );
            assert!(
                label.x >= cell.x && label.right() <= cell.right(),
                "{pct}%: {label:?} in {cell:?}"
            );
            assert!(
                label.bottom() < cell.bottom(),
                "{pct}%: two lines fit the cell"
            );
        }
    }

    /// A grown header is taller and draws its rule lower, clear of the grown title; the rule keeps its length.
    #[test]
    fn a_grown_group_header_draws_its_rule_under_the_title() {
        const EXPECTED: [(u32, i32, i32); 2] = [(150, 48, 28), (200, 62, 36)];
        let mut sv = iconed(2, Mode::Tiles);
        sv.groups = vec![Group {
            title: "Folders".into(),
            first: 0,
        }];
        for (pct, header_h, rule_y) in EXPECTED {
            let (c, lay, theme) = render_at(&sv, Rect::new(0, 0, 900, 400), pct);
            let cs = &theme.colors;
            let h = lay.headers[0];
            assert_eq!(h.height, header_h, "{pct}%");
            assert_eq!(lay.cells[0].y, h.bottom(), "{pct}%");
            assert_eq!(
                c.get(h.x, h.y + rule_y),
                cs.gradient_active_title.argb(),
                "{pct}%"
            );
            assert_eq!(c.get(h.x, h.y + rule_y + 1), cs.window.argb(), "{pct}%");
            assert_eq!(
                c.get(h.x, h.y + EXPECTED_GROUP_RULE_Y),
                cs.window.argb(),
                "{pct}%"
            );
            let ink = cs.window_text.argb();
            let title_rows: Vec<i32> = (h.y..h.bottom())
                .filter(|&y| (h.x..h.x + 120).any(|x| c.get(x, y) == ink))
                .collect();
            assert!(!title_rows.is_empty(), "{pct}%: the title draws");
            assert!(
                title_rows.iter().all(|&y| y < h.y + rule_y),
                "{pct}%: {title_rows:?}"
            );
            assert_ne!(
                c.get(h.x + GROUP_LINE_W + 1, h.y + rule_y),
                cs.gradient_active_title.argb()
            );
        }
    }

    /// Arrow keys, hits and the wheel follow the grown grid: at 200% a row holds half as many icons.
    #[test]
    fn arrow_keys_and_the_wheel_follow_the_grown_rows() {
        use crate::controls::shellview::{Hit, hit, scroll, step};
        use crate::selection::Mods;
        let r = Rect::new(0, 0, 3 * 150 + 2 * FIELD_PAD + 20, 200);
        for (pct, per_row, pitch) in [(100, 6, 75), (200, 3, 112)] {
            let mut sv = iconed(30, Mode::Icons);
            sv.selection.set_only(Some(0));
            let cm = Theme::default().with_text_size(pct).controls();
            let lay = layout(&sv, r, &cm);
            assert!(step(
                &mut sv,
                iconview::IconMove::Down,
                &lay,
                Mods::default()
            ));
            assert_eq!(sv.selection.focus, Some(per_row), "{pct}%");
            assert!(step(
                &mut sv,
                iconview::IconMove::Right,
                &lay,
                Mods::default()
            ));
            assert_eq!(sv.selection.focus, Some(per_row + 1), "{pct}%");
            let c = lay.cells[per_row + 1];
            assert_eq!(
                hit(&sv, &lay, c.right() - 1, c.y + 1),
                Hit::Entry(per_row + 1)
            );
            sv.top = 0;
            let lay = layout(&sv, r, &cm);
            assert!(scroll(&mut sv, 1, &lay));
            assert_eq!(sv.top, pitch, "{pct}%: a wheel step is one row");
        }
    }

    #[test]
    fn renaming_in_icons_draws_iconviews_edit_box() {
        let mut sv = iconed(1, Mode::Icons);
        sv.entries[0].name = "item0".into();
        sv.edit = Some((0, TextEdit::new("item0", true)));
        let (c, lay, theme) = render(&sv, Rect::new(0, 0, 300, 300));
        let (text, px) = rend();
        let _ = px;
        let box_r =
            iconview::edit_layout(&theme, &text, &sv.edit.as_ref().unwrap().1, iconview::CELL).rect;
        let cell = lay.cells[0];
        let black = Color::rgb(0, 0, 0).argb();
        assert_eq!(
            c.get(cell.x + box_r.x, cell.y + box_r.y),
            black,
            "the box's top-left border pixel"
        );
        assert_eq!(
            c.get(cell.x + box_r.x + 1, cell.y + box_r.y + 1),
            Color::rgb(255, 255, 255).argb(),
            "white inside it"
        );
    }
}
