//! The shell view's icon-mode grid (Icons, Tiles, Thumbnails): the row-major cell flow, group headers and cell drawing. `ShellView::top` is a px offset here, not a row count.

use crate::classic::widgets::draw_focus_ring;
use crate::controls::iconview;
use crate::controls::listview::{GroupRule, draw_group_header};
use crate::controls::shellview::{Entry, Group, Layout, Mode, ShellView, draw_rename};
use crate::look::Paint;
use crate::look::widgets::item;
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use crate::textedit::Caret;
use crate::theme::{ColorScheme, Theme};
use std::borrow::Cow;

/// A thumbnail's frame, border included.
pub const THUMB_FRAME: i32 = 96;
pub const THUMB_CELL: (i32, i32) = (125, 139);
pub(crate) const THUMB_FRAME_X: i32 = 17;
pub(crate) const THUMB_FRAME_Y: i32 = 3;
pub(crate) const THUMB_LABEL_GAP: i32 = 2;
pub(crate) const THUMB_RING: i32 = 3;
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

fn grid_top(cell: (i32, i32)) -> i32 {
    if cell == TILE_CELL {
        TILE_TOP
    } else {
        GRID_TOP
    }
}

/// The cell `mode` flows on; `List` and `Details` have their own layout and are not asked.
pub(crate) fn mode_cell(mode: Mode) -> (i32, i32) {
    match mode {
        Mode::Thumbnails => THUMB_CELL,
        Mode::Tiles => TILE_CELL,
        _ => ICON_CELL,
    }
}

/// The icon modes' row-major flow: one rect per entry on a `cell` grid filling `body`'s width, a full-width header rect before each group's first entry (`GROUP_GAP` of air first, unless it opens the view). A grouped view's cells start `GROUP_INDENT` in. `top` is the px already scrolled away, subtracted from every rect.
pub(crate) fn grid(
    cell: (i32, i32),
    body: Rect,
    count: usize,
    groups: &[Group],
    top: i32,
) -> (Vec<Rect>, Vec<Rect>) {
    let (mut cells, mut headers) = (Vec::with_capacity(count), Vec::new());
    flow(
        cell,
        body,
        count,
        groups,
        top,
        |r| cells.push(r),
        |r| headers.push(r),
    );
    (cells, headers)
}

pub(crate) fn grid_height(cell: (i32, i32), body: Rect, count: usize, groups: &[Group]) -> i32 {
    flow(cell, body, count, groups, 0, |_| {}, |_| {})
}

/// The flow `grid` and `grid_height` share; `groups` is sorted by `first`, so one cursor walks it.
fn flow(
    cell: (i32, i32),
    body: Rect,
    count: usize,
    groups: &[Group],
    top: i32,
    mut on_cell: impl FnMut(Rect),
    mut on_header: impl FnMut(Rect),
) -> i32 {
    let indent = if groups.is_empty() { 0 } else { GROUP_INDENT };
    let cols = ((body.width - indent) / cell.0).max(1);
    let mut y = grid_top(cell);
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
            on_header(Rect::new(
                body.x,
                body.y + y - top,
                body.width,
                GROUP_HEADER_H,
            ));
            y += GROUP_HEADER_H;
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

pub(crate) fn block_h(n: usize, px: u32) -> i32 {
    (n.max(1) as i32 - 1) * LINE_PITCH + px as i32
}

pub(crate) fn widths(text: &TextRenderer, lines: &[&str], px: u32) -> (i32, Vec<i32>) {
    let each: Vec<i32> = lines.iter().map(|l| text.measure(l, false, px)).collect();
    (each.iter().copied().max().unwrap_or(0), each)
}

pub(crate) fn thumb_frame(cell: Rect) -> Rect {
    Rect::new(
        cell.x + THUMB_FRAME_X,
        cell.y + THUMB_FRAME_Y,
        THUMB_FRAME,
        THUMB_FRAME,
    )
}

pub(crate) fn thumb_lines(text: &TextRenderer, name: &str, px: u32) -> Vec<String> {
    iconview::wrap_label(text, name, THUMB_FRAME, px)
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

/// One tile: the 48 px icon, then the name and `detail` lines in one block, filled with the highlight when selected. `cell` and `label` are already scaled and offset into the canvas.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_tile(
    c: &mut Canvas,
    text: &TextRenderer,
    cs: &ColorScheme,
    e: &Entry,
    cell: Rect,
    label: Rect,
    sel: (bool, bool),
    px: u32,
    s: i32,
) {
    let (selected, ring) = sel;
    if let Some(icon) = entry_icon(e, Mode::Tiles) {
        let (ix, iy) = (cell.x + TILE_ICON_OFF.0 * s, cell.y + TILE_ICON_OFF.1 * s);
        c.blit_over(ix, iy, &icon);
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
            label.y + k as i32 * LINE_PITCH * s,
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

/// One thumbnail: the framed image (or the 32 px icon centred in it), a `THUMB_RING` highlight ring when selected, and the wrapped label under it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_thumb(
    c: &mut Canvas,
    text: &TextRenderer,
    cs: &ColorScheme,
    e: &Entry,
    cell: Rect,
    label: Rect,
    sel: (bool, bool),
    px: u32,
    s: i32,
) {
    let (selected, ring) = sel;
    let frame = Rect::new(
        cell.x + THUMB_FRAME_X * s,
        cell.y + THUMB_FRAME_Y * s,
        THUMB_FRAME * s,
        THUMB_FRAME * s,
    );
    if selected {
        c.fill_rect(frame.inset(-(THUMB_RING * s)), cs.hilight);
    }
    c.fill_rect(frame, cs.window);
    c.ring_t(frame, cs.button_face, cs.button_face, s);
    if let Some(img) = entry_icon(e, Mode::Thumbnails) {
        let ix = frame.x + (frame.width - img.width as i32) / 2;
        let iy = frame.y + (frame.height - img.height as i32) / 2;
        c.blit_over(ix, iy, &img);
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
    for (k, line) in thumb_lines(text, &e.name, px).iter().enumerate() {
        let r = Rect::new(
            label.x,
            label.y + k as i32 * LINE_PITCH * s,
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
            let title_box = (GROUP_TITLE_Y * s, px as i32 + 3 * s);
            let rule = GroupRule {
                from: cs.gradient_active_title,
                to: cs.window,
                width: GROUP_LINE_W,
            };
            draw_group_header(&mut bc, text, cs, r, &g.title, title_box, &rule, px, s);
        }
    }
    let theme = cell_theme(p, px);
    for (i, e) in sv.entries.iter().enumerate() {
        let Some(&cell) = lay.cells.get(i) else {
            break;
        };
        if off(cell).intersect(&all).is_empty() {
            continue;
        }
        let label = off(label_of(sv.mode, cell, e, text, px));
        let cell = off(cell);
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
                draw_tile(&mut bc, text, cs, e, cell, label, (selected, ring), px, s);
                if editing {
                    draw_rename(&mut bc, text, p, sv, i, label, px);
                }
            }
            _ => {
                draw_thumb(&mut bc, text, cs, e, cell, label, (selected, ring), px, s);
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

/// The label rect for an icon-mode cell: what `draw` highlights and where a rename box goes.
pub(crate) fn label_of(mode: Mode, cell: Rect, e: &Entry, text: &TextRenderer, px: u32) -> Rect {
    match mode {
        Mode::Icons => {
            let r = iconview::label_layout(text, &e.name, px, 1).rect;
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
            let h = block_h(shown.len(), px);
            Rect::new(
                cell.x + TILE_TEXT_X,
                cell.y + tile_text_top(shown.len()),
                widest,
                h,
            )
        }
        _ => {
            let frame = thumb_frame(cell);
            let lines = thumb_lines(text, &e.name, px);
            let (widest, _) = widths(
                text,
                &lines.iter().map(String::as_str).collect::<Vec<_>>(),
                px,
            );
            let h = block_h(lines.len(), px);
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

    #[test]
    fn renaming_in_icons_draws_iconviews_edit_box() {
        let mut sv = iconed(1, Mode::Icons);
        sv.entries[0].name = "item0".into();
        sv.edit = Some((0, TextEdit::new("item0", true)));
        let (c, lay, theme) = render(&sv, Rect::new(0, 0, 300, 300));
        let (text, px) = rend();
        let _ = px;
        let box_r = iconview::edit_layout(&theme, &text, &sv.edit.as_ref().unwrap().1).rect;
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
