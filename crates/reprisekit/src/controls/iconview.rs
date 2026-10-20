//! The icon view: the desktop grid of 75 px cells (grown with the text size, [`Cell::grown`]),
//! each an icon over a wrapped label, with selection, shortcut arrow, in-place rename box and
//! arrow-key rules. Pure drawing and geometry; the host decides what the icons stand for.

use std::ops::Range;

use crate::icon;
use crate::look::widgets::item;
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use crate::textedit::{Caret, Preedit, TextEdit};
use crate::theme::{Color, TextUnits, Theme};

/// Grid cell edge, the icon's inset from its cell's top edge, and icon size, in logical px.
pub const CELL: i32 = 75;
pub const ICON_TOP: i32 = 2;
pub const ICON_PX: i32 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub w: i32,
    pub h: i32,
}

impl Cell {
    pub const fn square(n: i32) -> Cell {
        Cell { w: n, h: n }
    }

    /// The desktop's cell with the label's room grown by `u`; the icon keeps its size and place.
    pub fn grown(u: TextUnits) -> Cell {
        Cell {
            w: u.len(CELL),
            h: LABEL_TOP + u.len(CELL - LABEL_TOP),
        }
    }
}

/// The label's top in its cell, 4 px under the icon.
const LABEL_TOP: i32 = ICON_TOP + ICON_PX + 4;

pub fn rows_per_column(cell: Cell, area: Rect) -> usize {
    (area.height / cell.h).max(1) as usize
}

pub fn columns(cell: Cell, area: Rect) -> usize {
    (area.width / cell.w).max(1) as usize
}

pub fn cell_rect(cell: Cell, area: Rect, col: i32, row: i32) -> Rect {
    Rect::new(area.x + col * cell.w, area.y + row * cell.h, cell.w, cell.h)
}

pub fn cell_at(cell: Cell, area: Rect, x: i32, y: i32) -> (i32, i32) {
    let col = ((x - area.x) / cell.w).clamp(0, columns(cell, area) as i32 - 1);
    let row = ((y - area.y) / cell.h).clamp(0, rows_per_column(cell, area) as i32 - 1);
    (col, row)
}

/// The cell an icon saved at `saved`, a cell of the 100% grid, takes in a grid of `cell`s: the
/// one holding the saved cell's centre. Saved positions stay in 100% cells, so they survive the
/// grid growing with the text size.
pub fn from_saved(cell: Cell, (c, r): (i32, i32)) -> (i32, i32) {
    (
        (c * CELL + CELL / 2).div_euclid(cell.w),
        (r * CELL + CELL / 2).div_euclid(cell.h),
    )
}

/// The 100% cell to save for an icon at `at` in a grid of `cell`s: the one holding its centre,
/// which [`from_saved`] maps back to `at`.
pub fn to_saved(cell: Cell, (c, r): (i32, i32)) -> (i32, i32) {
    (
        (c * cell.w + cell.w / 2).div_euclid(CELL),
        (r * cell.h + cell.h / 2).div_euclid(CELL),
    )
}

/// The icons `names` in a grid of `cell`s over `area`: each at its saved position
/// ([`from_saved`]), or the free cell nearest it when an earlier icon took that cell; the rest,
/// and those saved past the grid's end, flow into the free cells down the columns.
pub fn layout_free(
    cell: Cell,
    area: Rect,
    names: &[&str],
    positions: &std::collections::BTreeMap<String, (i32, i32)>,
) -> Vec<Rect> {
    let (cols, rows) = (
        columns(cell, area) as i32,
        rows_per_column(cell, area) as i32,
    );
    let mut taken = std::collections::HashSet::new();
    let mut cells: Vec<Option<(i32, i32)>> = names
        .iter()
        .map(|n| {
            let &saved = positions.get(*n)?;
            let (c, r) = from_saved(cell, saved);
            if saved.0 < 0 || saved.1 < 0 || c >= cols || r >= rows {
                return None;
            }
            let at = if taken.contains(&(c, r)) {
                free_cell_near(cell, area, &taken, (c, r))
            } else {
                (c, r)
            };
            taken.insert(at);
            Some(at)
        })
        .collect();
    let mut next = 0;
    for c in cells.iter_mut().filter(|c| c.is_none()) {
        let mut free = (next / rows, next % rows);
        while taken.contains(&free) {
            next += 1;
            free = (next / rows, next % rows);
        }
        taken.insert(free);
        *c = Some(free);
        next += 1;
    }
    cells
        .into_iter()
        .map(|c| {
            let (col, row) = c.unwrap_or((0, 0));
            cell_rect(cell, area, col, row)
        })
        .collect()
}

pub fn free_cell_near(
    cell: Cell,
    area: Rect,
    taken: &std::collections::HashSet<(i32, i32)>,
    want: (i32, i32),
) -> (i32, i32) {
    let (cols, rows) = (
        columns(cell, area) as i32,
        rows_per_column(cell, area) as i32,
    );
    let total = cols * rows;
    let start = want.0 * rows + want.1;
    (0..total)
        .map(|k| {
            let i = (start + k).rem_euclid(total);
            (i / rows, i % rows)
        })
        .find(|c| !taken.contains(c))
        .unwrap_or(want)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconMove {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
}

/// The icon `dir` selects from `current` among icons laid out at `cells`: the
/// nearest (earlier on a tie) inside a 90-degree cone in that direction; nothing
/// in the cone keeps the selection. Home/End pick first/last. With nothing
/// selected the first icon (last for Up/Left/End). None only when there are no icons.
pub fn move_selection(current: Option<usize>, cells: &[Rect], dir: IconMove) -> Option<usize> {
    let count = cells.len();
    if count == 0 {
        return None;
    }
    let Some((i, &from)) = current.and_then(|i| Some((i, cells.get(i)?))) else {
        return Some(match dir {
            IconMove::Up | IconMove::Left | IconMove::End => count - 1,
            _ => 0,
        });
    };
    let ahead = |c: &Rect| -> Option<i64> {
        let (dx, dy) = ((c.x - from.x) as i64, (c.y - from.y) as i64);
        let (forward, lateral) = match dir {
            IconMove::Right => (dx, dy),
            IconMove::Left => (-dx, dy),
            IconMove::Down => (dy, dx),
            IconMove::Up => (-dy, dx),
            IconMove::Home | IconMove::End => unreachable!(),
        };
        (forward > 0 && lateral.abs() <= forward).then_some(dx * dx + dy * dy)
    };
    match dir {
        IconMove::Home => Some(0),
        IconMove::End => Some(count - 1),
        _ => cells
            .iter()
            .enumerate()
            .filter_map(|(j, c)| ahead(c).map(|d| (d, j)))
            .min()
            .map(|(_, j)| j)
            .or(Some(i)),
    }
}

pub fn shortcut_overlay(scale: i32) -> Canvas {
    let s = scale.max(1);
    let mut c = Canvas::new(11 * s as u32, 11 * s as u32);
    crate::art::icons::SHORTCUT.paint(&mut c, 0, 0, s);
    c
}

pub fn draw_shortcut_arrow(c: &mut Canvas, x: i32, bottom: i32, s: i32) {
    let ov = shortcut_overlay(s);
    c.blit_over(x, bottom - ov.height as i32, &ov);
}

pub fn with_shortcut_arrow(mut icon: Canvas) -> Canvas {
    let h = icon.height as i32;
    draw_shortcut_arrow(&mut icon, 0, h, 1);
    icon
}

pub fn layout(cell: Cell, area: Rect, count: usize) -> Vec<Rect> {
    let rows = rows_per_column(cell, area);
    (0..count)
        .map(|i| cell_rect(cell, area, (i / rows) as i32, (i % rows) as i32))
        .collect()
}

pub fn wrap_label(text: &TextRenderer, name: &str, max_w: i32, size_px: u32) -> Vec<String> {
    let words: Vec<&str> = name.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut first = String::new();
    let mut i = 0;
    while i < words.len() {
        let cand = if first.is_empty() {
            words[i].to_string()
        } else {
            format!("{first} {}", words[i])
        };
        if !first.is_empty() && text.measure(&cand, false, size_px) > max_w {
            break;
        }
        first = cand;
        i += 1;
    }
    let rest = words[i..].join(" ");
    let fit = |s: &str| text.ellipsize(s, max_w, false, size_px).into_owned();
    let mut lines = vec![fit(&first)];
    if !rest.is_empty() {
        lines.push(fit(&rest));
    }
    lines
}

/// Word-wrap `name` for the rename box: the byte range of each line, nothing cut
/// short, a too-wide word on its own line. Lines break at a single space, which
/// belongs to neither line. Empty text is one empty line.
pub fn wrap_edit(text: &TextRenderer, name: &str, max_w: i32, size_px: u32) -> Vec<Range<usize>> {
    let mut lines = Vec::new();
    let (mut start, mut end) = (0, 0);
    let word_ends = name.match_indices(' ').map(|(i, _)| i).chain([name.len()]);
    for word_end in word_ends {
        if end > start && text.measure(&name[start..word_end], false, size_px) > max_w {
            lines.push(start..end);
            start = end + 1;
        }
        end = word_end;
    }
    lines.push(start..end);
    lines
}

pub struct EditLayout {
    pub rect: Rect,
    pub step: i32,
    pub lines: Vec<EditLine>,
}

pub struct EditLine {
    pub text: String,
    pub start: usize,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub run: i32,
}

impl EditLine {
    pub fn at(&self, text: &TextRenderer, font: u32, b: usize) -> i32 {
        let end = self.start + self.text.len();
        if b > end {
            self.x + self.run
        } else {
            let i = self.text.floor_char_boundary(b - self.start);
            self.x + text.measure(&self.text[..i], false, font)
        }
    }
}

/// The rename box for `edit` in a `w` px wide cell at `theme.scale`: text wrapped and
/// centred in a white box with a 1 px border, never narrower than 30 px.
pub fn edit_layout(theme: &Theme, text: &TextRenderer, edit: &TextEdit, w: i32) -> EditLayout {
    edit_layout_at(theme.scale.max(1), theme.fonts.size_px, text, edit, w)
}

/// [`edit_layout`] at scale `s` with `font` px text.
pub fn edit_layout_at(
    s: i32,
    font: u32,
    text: &TextRenderer,
    edit: &TextEdit,
    w: i32,
) -> EditLayout {
    let size = w * s;
    let step = font as i32 + 2 * s;
    let ranges = wrap_edit(text, &edit.text, size - 8 * s, font);
    let widths: Vec<i32> = ranges
        .iter()
        .map(|r| text.measure(&edit.text[r.clone()], false, font))
        .collect();
    let widest = widths.iter().copied().max().unwrap_or(0);
    let rect_x = (size + 2 * s - widest) / 2 - 2 * s;
    let rect = Rect::new(
        rect_x - 3 * s,
        (ICON_TOP + ICON_PX + 3) * s,
        (widest + 12 * s).max(30 * s),
        ranges.len() as i32 * step + 4 * s,
    );
    let space = text.measure(" ", false, font);
    let mut lines = Vec::new();
    for (i, r) in ranges.into_iter().enumerate() {
        let last = i + 1 == widths.len();
        let run = widths[i] + if last { 0 } else { space };
        lines.push(EditLine {
            start: r.start,
            text: edit.text[r].to_string(),
            x: rect.x + s + (rect.width - 2 * s - run) / 2,
            y: rect.y + 2 * s + i as i32 * step,
            width: widths[i],
            run,
        });
    }
    EditLayout { rect, step, lines }
}

impl EditLayout {
    pub fn shift(&mut self, dx: i32) {
        self.rect.x += dx;
        for l in &mut self.lines {
            l.x += dx;
        }
    }

    pub fn index_at(&self, text: &TextRenderer, font: u32, x: i32, y: i32) -> usize {
        let Some(line) = self
            .lines
            .iter()
            .find(|l| y < l.y + self.step)
            .or(self.lines.last())
        else {
            return 0;
        };
        line.start + crate::textedit::index_at(text, &line.text, x - line.x, font)
    }
}

/// One desktop cell at `theme.scale`, grown with `theme.text` ([`Cell::grown`]): the icon
/// (already sized to `ICON_PX * scale`) near the top, the name below on up to two centred lines. A selected
/// cell blends the icon with `hilight`, fills the label in `hilight_text` and,
/// once the keyboard has been used, draws the dotted focus ring. A look with its
/// own desktop box ([`crate::style::Part::DesktopItem`]) draws that instead.
/// `rename` draws the in-progress edit box in place of the label, with `preedit`
/// wrapped in at the caret. A box wider than the cell returns a wider canvas
/// with the cell centred, so the caller offsets it by half the difference.
#[allow(clippy::too_many_arguments)]
pub fn draw_cell(
    theme: &Theme,
    text: &TextRenderer,
    name: &str,
    icon: Option<&Canvas>,
    shortcut: bool,
    selected: Option<Selection>,
    rename: Option<(&TextEdit, Caret)>,
    preedit: Option<&Preedit>,
) -> Canvas {
    let cell = Cell::grown(theme.text);
    let s = theme.scale.max(1);
    let size = cell.w * s;
    let extra = rename.map_or(0, |(edit, _)| {
        let r = edit_layout(theme, text, edit, cell.w).rect;
        (-r.x).max(r.right() - size).max(0)
    });
    let mut c = Canvas::new((size + 2 * extra) as u32, (cell.h * s) as u32);
    c.set_preedit(preedit.cloned());
    draw_cell_at(
        &mut c,
        (extra, 0),
        cell,
        theme,
        text,
        name,
        icon,
        shortcut,
        selected,
        rename,
    );
    c
}

/// [`draw_cell`] into `c` at device px `pos`, in a `cell` sized in logical px.
#[allow(clippy::too_many_arguments)]
pub fn draw_cell_at(
    c: &mut Canvas,
    pos: (i32, i32),
    cell: Cell,
    theme: &Theme,
    text: &TextRenderer,
    name: &str,
    icon: Option<&Canvas>,
    shortcut: bool,
    selected: Option<Selection>,
    rename: Option<(&TextEdit, Caret)>,
) {
    let (x0, y0) = pos;
    let s = theme.scale.max(1);
    let cs = &theme.colors;
    let size = cell.w * s;
    let icon_px = ICON_PX * s;
    let fallback;
    let icon = match icon {
        Some(i) => i,
        None => {
            fallback = icon::fallback(theme, icon_px);
            &fallback
        }
    };
    let boxed = selected.is_some() && rename.is_none() && {
        let l = label_layout(text, name, theme.fonts.size_px, s, cell.w).rect;
        let r = Rect::new(x0, y0, size, (l.bottom() + 2 * s).min(cell.h * s));
        item::draw_desktop_item(c, &theme.paint(), r)
    };
    let selected = selected.filter(|_| !boxed);
    let tinted;
    let icon = match selected {
        Some(_) => {
            tinted = tint_selected(icon, cs.hilight);
            &tinted
        }
        None => icon,
    };
    let ix = (size - icon.width as i32 + 1) / 2;
    c.blit_over(x0 + ix, y0 + ICON_TOP * s, icon);
    if shortcut {
        draw_shortcut_arrow(c, x0 + ix, y0 + (ICON_TOP + ICON_PX) * s, s);
    }

    let font = theme.fonts.size_px;
    let line_h = font as i32 + 3 * s;

    if let Some((own, caret_state)) = rename {
        let preedit = c.preedit().filter(|_| caret_state.focused()).cloned();
        let composed;
        let (edit, run, bar_on) = match &preedit {
            Some(p) => {
                let (e, run, bar_on) = p.compose(own);
                composed = e;
                (&composed, Some(run), bar_on)
            }
            None => (own, None, true),
        };
        let white = cs.window;
        let black = cs.window_text;
        let mut lay = edit_layout(theme, text, edit, cell.w);
        lay.shift(x0);
        let r = Rect::new(lay.rect.x, y0 + lay.rect.y, lay.rect.width, lay.rect.height);
        c.fill_rect(r, white);
        c.ring_t(r, black, black, s);
        let sel = edit.selection();
        let mut caret = None;
        for l in &lay.lines {
            let end = l.start + l.text.len();
            let at = |b: usize| l.at(text, font, b);
            let ly = y0 + l.y;
            let (a, b) = (sel.start.max(l.start), sel.end.min(end + 1));
            if a < b {
                c.fill_rect(Rect::new(at(a), ly, at(b) - at(a), lay.step), cs.hilight);
            }
            text.draw(
                c,
                Rect::new(l.x, ly, l.width, line_h),
                &l.text,
                false,
                black,
                Align::Left,
                font,
            );
            if a < b && a < end {
                let b = b.min(end);
                text.draw(
                    c,
                    Rect::new(at(a), ly, at(b) - at(a), line_h),
                    &l.text[a - l.start..b - l.start],
                    false,
                    cs.hilight_text,
                    Align::Left,
                    font,
                );
            }
            if let Some(run) = &run {
                let (a, b) = (run.start.max(l.start), run.end.min(end));
                if a < b {
                    let base = text
                        .baseline(Rect::new(l.x, ly, l.width, line_h), false, font)
                        .unwrap_or(ly + line_h - 3 * s);
                    c.fill_rect(Rect::new(at(a), base + s, at(b) - at(a), s), black);
                }
            }
            if caret.is_none() && edit.caret() <= end {
                caret = Some((at(edit.caret()), ly));
            }
        }
        let (cx, cy) = caret.unwrap_or((r.x + 5 * s, r.y + 2 * s));
        let bar = Rect::new(cx, cy, s, lay.step);
        if caret_state.shown() && bar_on {
            c.fill_rect(bar, black);
        }
        if caret_state.focused() {
            c.report_spot(own.spot(r, bar));
        }
        return;
    }

    let label = label_layout(text, name, font, s, cell.w);
    let l = label.rect;
    let rect = Rect::new(x0 + l.x, y0 + l.y, l.width, l.height);
    let fg = if selected.is_some() {
        c.fill_rect(rect, cs.hilight);
        cs.hilight_text
    } else {
        cs.desktop_text
    };
    for (i, line) in label.lines.iter().enumerate() {
        let y = rect.y + i as i32 * label.step;
        text.draw(
            c,
            Rect::new(x0 + line.x, y, line.width, line_h),
            &line.text,
            false,
            fg,
            Align::Left,
            font,
        );
    }
    if let Some(Selection {
        ring: true,
        origin: (ox, oy),
    }) = selected
    {
        let dot = cs.hilight.inverted();
        let origin = (ox - x0 / s, oy - y0 / s);
        crate::classic::widgets::draw_focus_ring_screen(c, rect, s, dot, origin);
    }
}

/// A `w` px wide cell's label at scale `s`, in the cell's coordinates: `name` wrapped to two
/// lines; `rect` is what a selection fills.
pub struct LabelLayout {
    pub rect: Rect,
    pub step: i32,
    pub lines: Vec<LabelLine>,
}

pub struct LabelLine {
    pub text: String,
    pub x: i32,
    pub width: i32,
}

pub fn label_layout(text: &TextRenderer, name: &str, px: u32, s: i32, w: i32) -> LabelLayout {
    let size = w * s;
    let step = px as i32 + 2 * s;
    let lines: Vec<LabelLine> = wrap_label(text, name, size - 4 * s, px)
        .into_iter()
        .map(|l| {
            let width = text.measure(&l, false, px);
            LabelLine {
                x: (size + 2 * s - width) / 2,
                width,
                text: l,
            }
        })
        .collect();
    let x0 = lines.iter().map(|l| l.x).min().unwrap_or(size / 2);
    let x1 = lines
        .iter()
        .map(|l| l.x + l.width)
        .max()
        .unwrap_or(size / 2);
    let rect = Rect::new(
        x0 - 2 * s,
        LABEL_TOP * s,
        x1 - x0 + 4 * s,
        lines.len() as i32 * step + 2 * s,
    );
    LabelLayout { rect, step, lines }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub ring: bool,
    pub origin: (i32, i32),
}

pub fn tint_selected(icon: &Canvas, hilight: Color) -> Canvas {
    let mut out = icon.clone();
    for p in &mut out.pixels {
        let a = *p >> 24;
        if a == 0 {
            continue;
        }
        let mix = |c: u32, h: u8| (c + (h as u32 * a + 127) / 255) / 2;
        let r = mix((*p >> 16) & 0xFF, hilight.r);
        let g = mix((*p >> 8) & 0xFF, hilight.g);
        let b = mix(*p & 0xFF, hilight.b);
        *p = a << 24 | r << 16 | g << 8 | b;
    }
    out
}

/// The side of the drag image's square in 100% px; it grows with the text size.
pub const DRAG_IMAGE_MAX: i32 = 256;

pub const DESKTOP_DRAG_MAX: i32 = 300;

/// The part of a desktop drag's `bounds` (every dragged cell) its image
/// shows: a [`DESKTOP_DRAG_MAX`] square centred on `pointer`, moved back
/// inside `bounds` where it would stick out, on each axis where `bounds`
/// is the longer. Icons wholly outside it are left out of the image.
pub fn desktop_drag_window(bounds: Rect, pointer: (i32, i32)) -> Rect {
    let axis = |lo: i32, len: i32, at: i32| {
        if len <= DESKTOP_DRAG_MAX {
            return (lo, len);
        }
        let start = (at - DESKTOP_DRAG_MAX / 2).clamp(lo, lo + len - DESKTOP_DRAG_MAX);
        (start, DESKTOP_DRAG_MAX)
    };
    let (x, w) = axis(bounds.x, bounds.width, pointer.0);
    let (y, h) = axis(bounds.y, bounds.height, pointer.1);
    Rect::new(x, y, w, h)
}

/// The strength of the desktop's drag image at `p`, over 256: half throughout
/// when nothing was cut off; a cut image is half at `hot` and fades linearly
/// to nothing at its edges, along an ellipse.
pub fn desktop_drag_strength(
    p: (i32, i32),
    hot: (i32, i32),
    size: (i32, i32),
    clipped: bool,
) -> u32 {
    const HALF: f32 = 128.0;
    if !clipped {
        return HALF as u32;
    }
    let reach = |d: i32, at: i32, len: i32| {
        let edge = if d < 0 { at } else { len - at };
        d as f32 / edge.max(1) as f32
    };
    let (dx, dy) = (p.0 - hot.0, p.1 - hot.1);
    let (nx, ny) = (reach(dx, hot.0, size.0), reach(dy, hot.1, size.1));
    let d = (nx * nx + ny * ny).sqrt();
    (HALF * (1.0 - d)).max(0.0).round() as u32
}

pub struct Ghost<'a> {
    pub at: (i32, i32),
    pub name: &'a str,
    pub icon: Option<&'a Canvas>,
    pub shortcut: bool,
}

/// The part of a drag of `cells` its image shows: a square [`DRAG_IMAGE_MAX`] 100% px across
/// centred on `pointer`. At 100% the square cuts through the cells at its edges; at a larger text
/// size `u` it keeps only the cells it holds whole, and cuts as at 100% when it holds none.
/// Returns the indices of the cells it shows and the rect it covers; None when no cell reaches it.
pub fn drag_cut(u: TextUnits, cells: &[Rect], pointer: (i32, i32)) -> Option<(Vec<usize>, Rect)> {
    let side = u.len(DRAG_IMAGE_MAX);
    let half = side / 2;
    let window = Rect::new(pointer.0 - half, pointer.1 - half, side, side);
    let whole: Vec<usize> = (0..cells.len())
        .filter(|&i| window.intersect(&cells[i]) == cells[i])
        .collect();
    let (shown, r) = if u.is_one() || whole.is_empty() {
        let bounds = cells.iter().copied().reduce(|a, b| a.union(&b))?;
        let r = bounds.intersect(&window);
        let shown = (0..cells.len())
            .filter(|&i| !cells[i].intersect(&r).is_empty())
            .collect();
        (shown, r)
    } else {
        let r = whole.iter().map(|&i| cells[i]).reduce(|a, b| a.union(&b))?;
        (whole, r)
    };
    (!r.is_empty()).then_some((shown, r))
}

/// The image a drag of several icons carries: every icon drawn unselected in
/// a `cell`, faded 1:2, cut as [`drag_cut`] cuts.
/// Returns the image and the pointer's place in it; None when no cell reaches it.
pub fn drag_image(
    theme: &Theme,
    text: &TextRenderer,
    ghosts: &[Ghost],
    cell: Cell,
    pointer: (i32, i32),
) -> Option<(Canvas, (i32, i32))> {
    let s = theme.scale.max(1);
    let cells: Vec<Rect> = (ghosts.iter())
        .map(|g| Rect::new(g.at.0, g.at.1, cell.w, cell.h))
        .collect();
    let (shown, r) = drag_cut(theme.text, &cells, pointer)?;
    let mut c = Canvas::new((r.width * s) as u32, (r.height * s) as u32);
    for g in shown.into_iter().map(|i| &ghosts[i]) {
        let pos = ((g.at.0 - r.x) * s, (g.at.1 - r.y) * s);
        draw_cell_at(
            &mut c, pos, cell, theme, text, g.name, g.icon, g.shortcut, None, None,
        );
    }
    c.fade(1, 2);
    Some((c, (pointer.0 - r.x, pointer.1 - r.y)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::textedit::EditKey;
    use crate::theme::FontConfig;

    #[test]
    fn free_layout_honours_positions_and_fills_the_rest_column_major() {
        use std::collections::BTreeMap;
        let area = Rect::new(0, 0, 400, 240); // 5 columns, 3 rows
        let square = Cell::square(CELL);
        let mut pos = BTreeMap::new();
        pos.insert("b".to_string(), (2, 1));
        pos.insert("c".to_string(), (2, 1)); // taken by b: the next free cell down
        pos.insert("d".to_string(), (9, 0)); // off the grid: flows
        let cells = layout_free(square, area, &["a", "b", "c", "d", "e"], &pos);
        assert_eq!(cells[1], cell_rect(square, area, 2, 1));
        assert_eq!(cells[2], cell_rect(square, area, 2, 2));
        assert_eq!(cells[0], cell_rect(square, area, 0, 0));
        assert_eq!(cells[3], cell_rect(square, area, 0, 1));
        assert_eq!(cells[4], cell_rect(square, area, 0, 2));
        assert_eq!(
            layout_free(square, area, &["a", "b"], &BTreeMap::new()),
            layout(square, area, 2)
        );
        pos.clear();
        pos.insert("a".to_string(), (0, 1));
        let cells = layout_free(square, area, &["a", "b", "c"], &pos);
        assert_eq!(cells[1], cell_rect(Cell::square(CELL), area, 0, 0));
        assert_eq!(cells[2], cell_rect(Cell::square(CELL), area, 0, 2));
    }

    #[test]
    fn the_desktop_drag_window_is_a_square_round_the_pointer_inside_the_cells() {
        let big = Rect::new(0, 0, 760, 1013);
        assert_eq!(
            desktop_drag_window(big, (341, 497)),
            Rect::new(191, 347, 300, 300)
        );
        assert_eq!(
            desktop_drag_window(big, (38, 17)),
            Rect::new(0, 0, 300, 300)
        );
        assert_eq!(
            desktop_drag_window(big, (569, 897)),
            Rect::new(419, 713, 300, 300)
        );
        assert_eq!(
            desktop_drag_window(Rect::new(0, 0, 150, 900), (37, 800)),
            Rect::new(0, 600, 150, 300)
        );
        let small = Rect::new(0, 450, 75, 150);
        assert_eq!(desktop_drag_window(small, (37, 467)), small);
    }

    #[test]
    fn a_cut_desktop_drag_image_fades_from_the_pointer_to_its_edges() {
        let (hot, size) = ((150, 150), (300, 300));
        assert_eq!(desktop_drag_strength(hot, hot, size, true), 128);
        assert_eq!(desktop_drag_strength((225, 150), hot, size, true), 64);
        assert_eq!(desktop_drag_strength((150, 75), hot, size, true), 64);
        assert_eq!(desktop_drag_strength((0, 150), hot, size, true), 0);
        assert_eq!(desktop_drag_strength((299, 299), hot, size, true), 0);
        assert_eq!(
            desktop_drag_strength((38 + 131, 17), (38, 17), size, true),
            64
        );
        assert_eq!(desktop_drag_strength((0, 0), hot, size, false), 128);
    }

    #[test]
    fn cell_at_clamps_into_the_grid_and_free_cell_near_skips_taken_ones() {
        let area = Rect::new(0, 100, 400, 300);
        assert_eq!(cell_at(Cell::square(CELL), area, 10, 110), (0, 0));
        assert_eq!(
            cell_at(Cell::square(CELL), area, CELL * 2 + 1, 100 + CELL + 1),
            (2, 1)
        );
        assert_eq!(cell_at(Cell::square(CELL), area, 9999, -50), (4, 0));
        let taken = std::collections::HashSet::from([(1, 0), (1, 1)]);
        assert_eq!(
            free_cell_near(Cell::square(CELL), area, &taken, (0, 0)),
            (0, 0)
        );
        assert_eq!(
            free_cell_near(Cell::square(CELL), area, &taken, (1, 0)),
            (1, 2)
        );
        assert_eq!(
            free_cell_near(Cell::square(CELL), area, &taken, (4, 2)),
            (4, 2)
        );
        assert_eq!(
            free_cell_near(Cell::square(CELL), area, &taken, (-1, 1)),
            (4, 1)
        );
    }

    #[test]
    fn arrow_navigation_walks_the_column_major_grid() {
        use IconMove::*;
        let cells = layout(Cell::square(CELL), Rect::new(0, 0, 400, 2 * CELL), 5);
        let mv = |cur, dir| move_selection(cur, &cells, dir);
        assert_eq!(mv(None, Down), Some(0));
        assert_eq!(mv(None, Up), Some(4));
        assert_eq!(mv(Some(0), Down), Some(1));
        assert_eq!(mv(Some(1), Down), Some(1));
        assert_eq!(mv(Some(1), Up), Some(0));
        assert_eq!(mv(Some(0), Right), Some(2));
        assert_eq!(mv(Some(2), Left), Some(0));
        assert_eq!(mv(Some(3), Right), Some(4));
        assert_eq!(mv(Some(4), Right), Some(4));
        assert_eq!(mv(Some(2), Home), Some(0));
        assert_eq!(mv(Some(0), End), Some(4));
        assert_eq!(move_selection(None, &[], Down), None);
    }

    #[test]
    fn arrow_navigation_from_a_focus_past_the_end_starts_over() {
        use IconMove::*;
        let cells = layout(Cell::square(CELL), Rect::new(0, 0, 400, 2 * CELL), 3);
        assert_eq!(move_selection(Some(7), &cells, Down), Some(0));
        assert_eq!(move_selection(Some(7), &cells, Up), Some(2));
    }

    #[test]
    fn arrow_navigation_follows_free_positions() {
        use IconMove::*;
        let area = Rect::new(0, 0, 800, 600);
        let at = |c, r| cell_rect(Cell::square(CELL), area, c, r);
        let cells = [at(0, 0), at(4, 0), at(0, 2), at(3, 3), at(0, 4), at(0, 5)];
        let mv = |cur, dir| move_selection(Some(cur), &cells, dir);
        assert_eq!(mv(0, Right), Some(1));
        assert_eq!(mv(0, Down), Some(2));
        assert_eq!(mv(2, Down), Some(4));
        assert_eq!(mv(3, Up), Some(1));
        assert_eq!(mv(3, Left), Some(2));
        assert_eq!(mv(1, Right), Some(1));
        assert_eq!(mv(3, Down), Some(3));
        assert_eq!(mv(5, Right), Some(3));
        assert_eq!(mv(3, Home), Some(0));
        assert_eq!(mv(3, End), Some(5));
    }

    #[test]
    fn layout_is_column_major_from_top_left() {
        let area = Rect::new(0, 0, 300, 200);
        assert!(layout(Cell::square(CELL), area, 0).is_empty());
        let one = layout(Cell::square(CELL), area, 1);
        assert_eq!(one, vec![Rect::new(0, 0, CELL, CELL)]);
        let nine = layout(Cell::square(CELL), area, 9);
        assert_eq!(nine.len(), 9);
        assert_eq!(nine[1], Rect::new(0, CELL, CELL, CELL));
        assert_eq!(nine[2], Rect::new(CELL, 0, CELL, CELL));
        assert_eq!(nine[8], Rect::new(4 * CELL, 0, CELL, CELL));
        let off = layout(Cell::square(CELL), Rect::new(100, 50, 300, 20), 2);
        assert_eq!(off[0], Rect::new(100, 50, CELL, CELL));
        assert_eq!(off[1], Rect::new(100 + CELL, 50, CELL, CELL));
    }

    fn text() -> TextRenderer {
        TextRenderer::load(&FontConfig::default())
    }

    fn label_pixels(c: &Canvas, s: i32) -> Vec<(i32, i32, u32)> {
        let y0 = (ICON_TOP + ICON_PX + 4) * s;
        (y0..CELL * s)
            .flat_map(|y| (0..CELL * s).map(move |x| (x, y)))
            .map(|(x, y)| (x, y, c.get(x, y)))
            .filter(|&(_, _, p)| p != 0)
            .collect()
    }

    #[test]
    fn wrap_label_splits_into_two_lines_and_truncates() {
        let t = text();
        let size = 11;
        let max_w = t.measure("Photos Etc", false, size) + 2;
        assert_eq!(wrap_label(&t, "Photos", max_w, size), vec!["Photos"]);
        assert_eq!(
            wrap_label(&t, "Photos Etc More", max_w, size),
            vec!["Photos Etc", "More"]
        );
        let lines = wrap_label(&t, "Photos Etc More Words That Never End", max_w, size);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].ends_with("..."), "second line was {:?}", lines[1]);
        assert!(t.measure(&lines[1], false, size) <= max_w);
        let lines = wrap_label(&t, "Supercalifragilisticexpialidocious", max_w, size);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].ends_with("..."));
        assert!(wrap_label(&t, "   ", max_w, size).is_empty());
    }

    #[test]
    fn draw_cell_unselected_uses_desktop_text_and_fallback_icon() {
        let theme = Theme::default();
        let t = text();
        let c = draw_cell(&theme, &t, "Notes", None, false, None, None, None);
        assert_eq!(c.width, CELL as u32);
        assert_eq!(c.height, CELL as u32);
        let ix = (CELL - ICON_PX + 1) / 2;
        assert_eq!(c.get(ix, ICON_TOP), theme.colors.button_dk_shadow.argb());
        let label = label_pixels(&c, 1);
        assert!(!label.is_empty());
        assert!(
            label
                .iter()
                .all(|&(_, _, p)| p == theme.colors.desktop_text.argb())
        );
        assert_eq!(c.get(0, 0), 0);
        assert_eq!(c.get(CELL - 1, CELL - 1), 0);
    }

    #[test]
    fn draw_cell_shortcut_puts_the_arrow_box_at_the_icons_bottom_left() {
        let theme = Theme::default();
        let t = text();
        let c = draw_cell(&theme, &t, "Vim", None, true, None, None, None);
        let ix = (CELL - ICON_PX + 1) / 2;
        let top = ICON_TOP + ICON_PX - 11;
        let grey = Color::rgb(128, 128, 128).argb();
        let black = Color::rgb(0, 0, 0).argb();
        let white = Color::rgb(255, 255, 255).argb();
        assert_eq!(c.get(ix, top), grey);
        assert_eq!(c.get(ix, top + 5), grey);
        assert_eq!(c.get(ix + 10, top), black);
        assert_eq!(c.get(ix + 5, top + 10), black);
        assert_eq!(c.get(ix + 1, top + 1), white);
        assert_eq!(c.get(ix + 4, top + 2), black);
        assert_eq!(c.get(ix + 4, top + 8), black);
        assert_eq!(c.get(ix + 3, top + 8), white);
        let plain = draw_cell(&theme, &t, "Vim", None, false, None, None, None);
        assert_ne!(plain.get(ix + 1, top + 1), white);
    }

    #[test]
    fn with_shortcut_arrow_boxes_the_icons_bottom_left() {
        let mut icon = Canvas::new(32, 32);
        icon.fill_rect(Rect::new(0, 0, 32, 32), Color::rgb(255, 0, 0));
        let c = with_shortcut_arrow(icon);
        assert_eq!((c.width, c.height), (32, 32));
        let grey = Color::rgb(128, 128, 128).argb();
        let black = Color::rgb(0, 0, 0).argb();
        let red = Color::rgb(255, 0, 0).argb();
        assert_eq!(c.get(0, 21), grey, "the box's top-left corner");
        assert_eq!(c.get(10, 21), black, "its top-right corner");
        assert_eq!(c.get(0, 31), black, "its bottom row");
        assert_eq!(c.get(4, 23), black, "the arrow head");
        assert_eq!(c.get(0, 20), red, "the icon above the box");
        assert_eq!(c.get(11, 31), red, "the icon right of the box");
    }

    #[test]
    fn draw_cell_selected_highlights_label_with_rect_and_ring() {
        if !crate::text::test_tahoma() {
            return;
        }
        let theme = Theme::default();
        let t = text();
        let cs = &theme.colors;
        let font = theme.fonts.size_px;
        let ring = Some(Selection {
            ring: true,
            origin: (0, 75),
        });
        let c = draw_cell(&theme, &t, "Notes", None, false, ring, None, None);
        let w = t.measure("Notes", false, font);
        let y0 = ICON_TOP + ICON_PX + 4;
        let x0 = (CELL + 2 - w) / 2 - 2;
        let x1 = x0 + w + 4;
        let hil = cs.hilight.argb();
        let dot = cs.hilight.inverted().argb();
        assert_eq!(c.get(x0 + 1, y0 + 2), hil);
        assert_eq!(c.get(x0 - 1, y0 + 2), 0);
        assert_eq!(c.get(x1 - 1, y0 + 2), hil);
        assert_eq!(c.get(x1, y0 + 2), 0);
        assert_eq!(c.get(x0 + 1, y0 + 14), hil);
        assert_eq!(c.get(x0 + 1, y0 + 15), 0);
        for x in x0..x1 {
            let want = if (x + 75 + y0) % 2 != 0 { dot } else { hil };
            assert_eq!(c.get(x, y0), want, "top row x {x}");
            let want = if (x + 75 + y0 + 14) % 2 != 0 {
                dot
            } else {
                hil
            };
            assert_eq!(c.get(x, y0 + 14), want, "bottom row x {x}");
        }
        for y in y0 + 1..y0 + 14 {
            let want = if (x0 + 75 + y) % 2 != 0 { dot } else { hil };
            assert_eq!(c.get(x0, y), want, "left column y {y}");
        }
        assert_eq!(c.get(x0 - 1, y0 - 1), 0);
        assert_eq!(c.get(x0, y0 - 1), 0);
        assert!(
            label_pixels(&c, 1)
                .iter()
                .all(|&(_, _, p)| p == hil || p == dot || p == cs.hilight_text.argb())
        );
        let no_ring = Some(Selection {
            ring: false,
            origin: (0, 75),
        });
        let c = draw_cell(&theme, &t, "Notes", None, false, no_ring, None, None);
        assert!((x0..x1).all(|x| c.get(x, y0) == hil));
    }

    #[test]
    fn draw_cell_two_line_label_is_one_rect_on_a_13_px_pitch() {
        let theme = Theme::default();
        let t = text();
        let cs = &theme.colors;
        let font = theme.fonts.size_px;
        let name = "My Network Places";
        let lines = wrap_label(&t, name, CELL - 4, font);
        assert_eq!(lines.len(), 2);
        let sel = Some(Selection {
            ring: false,
            origin: (0, 0),
        });
        let c = draw_cell(&theme, &t, name, None, false, sel, None, None);
        let y0 = ICON_TOP + ICON_PX + 4;
        let hil = cs.hilight.argb();
        let wide = lines
            .iter()
            .map(|l| t.measure(l, false, font))
            .max()
            .unwrap();
        let x0 = (CELL + 2 - wide) / 2 - 2;
        assert_eq!(c.get(x0, y0), hil);
        assert_eq!(c.get(x0, y0 + 27), hil);
        assert_eq!(c.get(x0, y0 + 28), 0);
        assert_eq!(c.get(x0 + wide + 3, y0 + 20), hil);
        assert_eq!(c.get(x0 + wide + 4, y0 + 20), 0);
        assert_eq!(
            label_layout(&t, name, font, 1, CELL).rect,
            Rect::new(x0, y0, wide + 4, 28)
        );
        let ink = cs.hilight_text.argb();
        let rows: Vec<i32> = (y0..y0 + 28)
            .filter(|&y| (0..CELL).any(|x| c.get(x, y) == ink))
            .collect();
        let r0 = rows[0];
        assert!(r0 >= y0 + 2 && r0 <= y0 + 4, "first ink row {r0} from {y0}");
        assert!(rows.contains(&(r0 + 13)));
        assert!(!rows.contains(&(r0 + 12)));
    }

    #[test]
    fn selected_icon_is_blended_half_with_the_highlight() {
        let theme = Theme::default();
        let t = text();
        let mut icon = Canvas::new(ICON_PX as u32, ICON_PX as u32);
        icon.fill_rect(Rect::new(0, 0, 8, 8), Color::rgb(200, 100, 0));
        let sel = Some(Selection {
            ring: false,
            origin: (0, 0),
        });
        let c = draw_cell(&theme, &t, "Notes", Some(&icon), false, sel, None, None);
        let ix = (CELL - ICON_PX + 1) / 2;
        let h = theme.colors.hilight;
        let want = Color::rgb(
            ((200 + h.r as u32) / 2) as u8,
            ((100 + h.g as u32) / 2) as u8,
            (h.b as u32 / 2) as u8,
        );
        assert_eq!(c.get(ix, ICON_TOP), want.argb());
        assert_eq!(c.get(ix + 20, ICON_TOP + 20), 0);
        let mut soft = Canvas::new(ICON_PX as u32, ICON_PX as u32);
        soft.pixels.fill(0x8080_8080);
        let tinted = tint_selected(&soft, h);
        assert_eq!(tinted.pixels[0] >> 24, 0x80);
    }

    #[test]
    fn draw_cell_at_matches_draw_cell_blitted() {
        let theme = Theme::default();
        let t = text();
        let ring = Some(Selection {
            ring: true,
            origin: (7, 12),
        });
        let long = TextEdit::new("Supercalifragilisticexpialidocious", false);
        let cases = [(ring, None), (None, Some((&long, Caret::Shown)))];
        for (sel, rename) in cases {
            let cell = draw_cell(
                &theme,
                &t,
                "My Network Places",
                None,
                true,
                sel,
                rename,
                None,
            );
            let pad = (cell.width as i32 - CELL) / 2;
            let mut want = Canvas::new(300, 120);
            want.blit_over(41 - pad, 30, &cell);
            let mut got = Canvas::new(300, 120);
            draw_cell_at(
                &mut got,
                (41, 30),
                Cell::square(CELL),
                &theme,
                &t,
                "My Network Places",
                None,
                true,
                sel,
                rename,
            );
            assert!(got.pixels == want.pixels, "rename {}", rename.is_some());
        }
    }

    #[test]
    fn draw_cell_rename_draws_the_edit_box() {
        let theme = Theme::default();
        let t = text();
        let font = theme.fonts.size_px;
        let name = "New Text Document";
        let edit = TextEdit::new(name, true);
        let c = draw_cell(
            &theme,
            &t,
            name,
            None,
            false,
            None,
            Some((&edit, Caret::Shown)),
            None,
        );
        let lines = wrap_edit(&t, name, CELL - 8, font);
        assert_eq!(lines, [0..8, 9..17]);
        let widest = t.measure("Document", false, font);
        let rect_x = (CELL + 2 - widest) / 2 - 2;
        let (bx, by, bw, bh) = (rect_x - 3, ICON_TOP + ICON_PX + 3, widest + 12, 2 * 13 + 4);
        let black = 0xFF000000;
        assert_eq!(c.get(bx, by), black);
        assert_eq!(c.get(bx + bw - 1, by + bh - 1), black);
        assert_eq!(c.get(bx + bw, by), 0);
        assert_eq!(c.get(bx + 1, by + 1), 0xFFFFFFFF);
        let hl = theme.colors.hilight.argb();
        let run1 = t.measure("New Text", false, font) + t.measure(" ", false, font);
        let x1 = bx + 1 + (bw - 2 - run1) / 2;
        assert_eq!(c.get(x1, by + 1), 0xFFFFFFFF);
        assert_eq!(c.get(x1 - 1, by + 2), 0xFFFFFFFF);
        assert_eq!(c.get(x1, by + 2), hl);
        assert_eq!(c.get(x1 + run1 - 1, by + 2), hl);
        assert_eq!(c.get(x1 + run1, by + 2), 0xFFFFFFFF);
        assert_eq!(c.get(x1, by + 14), hl);
        assert_eq!(c.get(x1 + 1, by + 15), hl, "the second line's run");
        assert_eq!(c.get(x1, by + 28), 0xFFFFFFFF);
        assert!((0..CELL).any(|x| c.get(x, by + 8) == theme.colors.hilight_text.argb()));
        let x2 = bx + 1 + (bw - 2 - widest) / 2;
        assert_eq!(c.get(x2 + widest, by + 15), black);
        assert_eq!(c.get(x2 + widest, by + 27), black);
        let c = draw_cell(
            &theme,
            &t,
            name,
            None,
            false,
            None,
            Some((&TextEdit::new("Re", false), Caret::Shown)),
            None,
        );
        assert!((0..CELL).all(|x| (by..by + 20).all(|y| c.get(x, y) != hl)));
        assert!((0..CELL).any(|x| c.get(x, by + 8) == black));
        let (bx, bw) = (0..CELL)
            .filter(|&x| c.get(x, by) == black)
            .fold((CELL, 0), |(x0, w), x| (x0.min(x), w + 1));
        assert_eq!(bw, 30);
        assert_eq!(c.get(bx, by + 16), black);
        assert_eq!(c.get(bx, by + 17), 0);
    }

    #[test]
    fn draw_cell_rename_shows_a_partial_selection_and_the_caret_on_its_line() {
        let theme = Theme::default();
        let t = text();
        let font = theme.fonts.size_px;
        let name = "New Text Document";
        let hl = theme.colors.hilight.argb();
        let black = 0xFF000000;
        let by = ICON_TOP + ICON_PX + 3;
        let mut edit = TextEdit::new(name, false);
        for _ in 0..4 {
            edit.key(EditKey::Left {
                select: true,
                word: false,
            });
        }
        let c = draw_cell(
            &theme,
            &t,
            name,
            None,
            false,
            None,
            Some((&edit, Caret::Shown)),
            None,
        );
        assert!((0..CELL).all(|x| c.get(x, by + 2) != hl));
        let widest = t.measure("Document", false, font);
        let bw = widest + 12;
        let bx = (CELL + 2 - widest) / 2 - 2 - 3;
        let x2 = bx + 1 + (bw - 2 - widest) / 2;
        let docu = t.measure("Docu", false, font);
        assert_eq!(c.get(x2 + docu - 1, by + 15), 0xFFFFFFFF);
        assert_eq!(c.get(x2 + docu + 1, by + 15), hl);
        assert_eq!(c.get(x2 + widest - 1, by + 15), hl);
        assert_eq!(c.get(x2 + docu, by + 15), black);
        assert_eq!(c.get(x2 + docu, by + 27), black);
        let mut edit = TextEdit::new(name, false);
        edit.key(EditKey::Home { select: false });
        let c = draw_cell(
            &theme,
            &t,
            name,
            None,
            false,
            None,
            Some((&edit, Caret::Shown)),
            None,
        );
        let run1 = t.measure("New Text", false, font) + t.measure(" ", false, font);
        let x1 = bx + 1 + (bw - 2 - run1) / 2;
        assert_eq!(c.get(x1, by + 2), black);
        assert_eq!(c.get(x1, by + 14), black);
        assert!((0..CELL).all(|x| c.get(x, by + 15) != hl));
    }

    #[test]
    fn edit_layout_maps_cell_points_to_offsets() {
        let theme = Theme::default();
        let t = text();
        let font = theme.fonts.size_px;
        let edit = TextEdit::new("New Text Document", false);
        let lay = edit_layout(&theme, &t, &edit, CELL);
        assert_eq!(lay.lines.len(), 2);
        let (l1, l2) = (&lay.lines[0], &lay.lines[1]);
        assert_eq!((l1.start, l2.start), (0, 9));
        assert_eq!(lay.index_at(&t, font, -100, l1.y + 3), 0);
        assert_eq!(lay.index_at(&t, font, 500, l1.y + 3), 8);
        assert_eq!(lay.index_at(&t, font, -100, l2.y + 3), 9);
        assert_eq!(lay.index_at(&t, font, 500, l2.y + 3), 17);
        assert_eq!(lay.index_at(&t, font, -100, -50), 0);
        assert_eq!(lay.index_at(&t, font, 500, 500), 17);
        let w = t.measure("New", false, font);
        assert_eq!(lay.index_at(&t, font, l1.x + w, l1.y), 3);
    }

    #[test]
    fn edit_lines_are_the_real_text_at_their_start_through_any_spaces() {
        let theme = Theme::default();
        let t = text();
        for name in [
            " é",
            " é.txt",
            "  lead",
            "trail  ",
            "a  b",
            "New  Text   Document ",
            " é é  é ",
            "Ünïcödé wörds  wrap ácross   lines ",
            "",
        ] {
            let edit = TextEdit::new(name, false);
            let lay = edit_layout(&theme, &t, &edit, CELL);
            let mut next = 0;
            for l in &lay.lines {
                assert_eq!(
                    l.start, next,
                    "{name:?}: each line starts past its predecessor's space"
                );
                assert_eq!(
                    name.get(l.start..l.start + l.text.len()),
                    Some(l.text.as_str()),
                    "{name:?}: the line is the text at its start"
                );
                next = l.start + l.text.len() + 1;
            }
            assert_eq!(next, name.len() + 1, "{name:?}: the lines cover the text");
            for (i, _) in name.char_indices().chain([(name.len(), ' ')]) {
                let mut edit = TextEdit::new(name, false);
                edit.place(i, false);
                draw_cell(
                    &theme,
                    &t,
                    name,
                    None,
                    false,
                    None,
                    Some((&edit, Caret::Shown)),
                    None,
                );
            }
        }
    }

    #[test]
    fn draw_cell_rename_widens_the_canvas_for_a_long_word_and_hides_a_blinked_caret() {
        let theme = Theme::default();
        let t = text();
        let long = "Reasdfghijklmnop";
        let edit = TextEdit::new(long, false);
        let lay = edit_layout(&theme, &t, &edit, CELL);
        assert!(lay.rect.right() > CELL, "the box overflows the cell");
        let c = draw_cell(
            &theme,
            &t,
            long,
            None,
            false,
            None,
            Some((&edit, Caret::Shown)),
            None,
        );
        let extra = (-lay.rect.x).max(lay.rect.right() - CELL);
        assert_eq!(c.width as i32, CELL + 2 * extra);
        let by = ICON_TOP + ICON_PX + 3;
        assert_eq!(c.get(lay.rect.x + extra, by), 0xFF000000);
        assert_eq!(c.get(lay.rect.right() - 1 + extra, by), 0xFF000000);
        let on = draw_cell(
            &theme,
            &t,
            "Re",
            None,
            false,
            None,
            Some((&TextEdit::new("Re", false), Caret::Shown)),
            None,
        );
        let off = draw_cell(
            &theme,
            &t,
            "Re",
            None,
            false,
            None,
            Some((&TextEdit::new("Re", false), Caret::Hidden)),
            None,
        );
        let blacks = |c: &Canvas| {
            (0..CELL)
                .filter(|&x| c.get(x, by + 8) == 0xFF000000)
                .count()
        };
        assert_eq!(blacks(&on), blacks(&off) + 1);
    }

    #[test]
    fn wrap_edit_never_cuts_and_grows_past_two_lines() {
        let t = text();
        let font = Theme::default().fonts.size_px;
        let wrap = |name| -> Vec<&str> {
            wrap_edit(&t, name, 67, font)
                .into_iter()
                .map(|r| &name[r])
                .collect()
        };
        assert_eq!(wrap(""), [""]);
        assert_eq!(wrap("Reports"), ["Reports"]);
        let name = "Photos Etc More Words That Never End";
        let lines = wrap(name);
        assert!(lines.len() > 2);
        assert_eq!(lines.join(" "), name);
        let long = "Supercalifragilisticexpialidocious";
        assert_eq!(wrap(long), [long]);
    }

    #[test]
    fn draw_cell_scales_with_theme() {
        let theme = Theme::default().scaled(2);
        let t = text();
        let mut icon = Canvas::new(64, 64);
        icon.fill_rect(Rect::new(0, 0, 64, 64), crate::theme::Color::rgb(1, 2, 3));
        let c = draw_cell(&theme, &t, "Notes", Some(&icon), false, None, None, None);
        assert_eq!(c.width, 150);
        assert_eq!(c.get(CELL, ICON_TOP * 2 + 1), 0xFF010203);
        assert!(!label_pixels(&c, 2).is_empty());
    }

    #[test]
    fn draw_cell_keeps_translucent_icon_pixels_translucent() {
        let theme = Theme::default();
        let t = text();
        let mut icon = Canvas::new(ICON_PX as u32, ICON_PX as u32);
        icon.pixels.fill(0x8080_8080);
        let c = draw_cell(&theme, &t, "Notes", Some(&icon), false, None, None, None);
        let ix = (CELL - ICON_PX + 1) / 2;
        assert_eq!(c.get(ix, ICON_TOP), 0x8080_8080);
    }

    #[test]
    fn layout_flows_a_non_square_cell() {
        let cell = Cell { w: 150, h: 17 };
        let area = Rect::new(0, 0, 450, 51); // 3 rows per column, 3 columns
        assert_eq!(rows_per_column(cell, area), 3);
        assert_eq!(columns(cell, area), 3);
        let four = layout(cell, area, 4);
        assert_eq!(four[0], Rect::new(0, 0, 150, 17));
        assert_eq!(four[1], Rect::new(0, 17, 150, 17));
        assert_eq!(four[2], Rect::new(0, 34, 150, 17));
        assert_eq!(four[3], Rect::new(150, 0, 150, 17));
    }

    #[test]
    fn square_cell_matches_the_desktop_grid() {
        let cell = Cell::square(CELL);
        let area = Rect::new(0, 0, 300, 200);
        assert_eq!(rows_per_column(cell, area), 2);
        assert_eq!(columns(cell, area), 4);
        assert_eq!(cell_at(cell, area, 80, 80), (1, 1));
    }

    /// At 200% the grid's cells are 150x112. A saved 100% cell maps to the grown cell holding its
    /// centre and back; a column of five saved icons stays a column, its neighbours that share a
    /// grown cell taking the next ones down; at 100% the mapping changes nothing.
    #[test]
    fn saved_positions_survive_the_grid_growing() {
        use std::collections::BTreeMap;
        let big = Cell::grown(TextUnits::new(22, 11));
        assert_eq!(big, Cell { w: 150, h: 112 });
        assert_eq!(Cell::grown(TextUnits::default()), Cell::square(CELL));
        for c in 0..12 {
            for r in 0..9 {
                assert_eq!(from_saved(big, to_saved(big, (c, r))), (c, r), "({c}, {r})");
                let square = Cell::square(CELL);
                assert_eq!(from_saved(square, (c, r)), (c, r));
                assert_eq!(to_saved(square, (c, r)), (c, r));
            }
        }
        assert_eq!(from_saved(big, (3, 5)), (1, 3));
        let area = Rect::new(0, 0, 1920, 1040);
        let names = ["a", "b", "c", "d", "e", "far"];
        let mut pos: BTreeMap<String, (i32, i32)> = (0..5)
            .map(|r| (names[r].to_string(), (0, r as i32)))
            .collect();
        pos.insert("far".into(), (20, 3));
        let cells = layout_free(big, area, &names, &pos);
        let at = |i: usize| cell_at(big, area, cells[i].x, cells[i].y);
        assert_eq!(
            (0..5).map(at).collect::<Vec<_>>(),
            [(0, 0), (0, 1), (0, 2), (0, 3), (0, 4)]
        );
        assert_eq!(at(5), from_saved(big, (20, 3)), "{:?}", at(5));
        assert_eq!(at(5), (10, 2));
    }

    /// A cell drawn at 200% text is the grown 150x112 with its label set at the grown font: the
    /// icon keeps its 32 px at the top, the name's ink stands under it, taller than at 100%.
    #[test]
    fn a_cell_grows_with_the_text_and_its_label_with_it() {
        let t = TextRenderer::load(&FontConfig::default());
        let ink_rows = |c: &Canvas| {
            (0..c.height as i32)
                .filter(|&y| (0..c.width as i32).any(|x| c.get(x, y) >> 24 > 0x80))
                .collect::<Vec<_>>()
        };
        let small = draw_cell(
            &Theme::default(),
            &t,
            "Notes",
            None,
            false,
            None,
            None,
            None,
        );
        let theme = Theme::default().with_form_text(200);
        let c = draw_cell(&theme, &t, "Notes", None, false, None, None, None);
        assert_eq!((c.width, c.height), (150, 112));
        let label = |c: &Canvas| {
            let rows = ink_rows(c);
            let below: Vec<i32> = rows.into_iter().filter(|&y| y >= LABEL_TOP).collect();
            below.last().unwrap_or(&0) - below.first().unwrap_or(&0)
        };
        assert!(
            label(&c) > label(&small) * 3 / 2,
            "the label stands {} rows at 200%, {} at 100%",
            label(&c),
            label(&small)
        );
    }

    fn ghost_cell(theme: &Theme, t: &TextRenderer, name: &str) -> Canvas {
        let mut c = draw_cell(theme, t, name, None, false, None, None, None);
        c.fade(1, 2);
        c
    }

    const DESKTOP: Cell = Cell::square(CELL);

    fn holds(img: &Canvas, cell: &Canvas, at: (i32, i32)) -> bool {
        (0..cell.height as i32)
            .all(|y| (0..cell.width as i32).all(|x| img.get(at.0 + x, at.1 + y) == cell.get(x, y)))
    }

    #[test]
    fn a_drag_image_stacks_two_icons_as_they_sit() {
        let (theme, t) = (Theme::default(), text());
        let ghosts = [
            Ghost {
                at: (100, 40),
                name: "a.txt",
                icon: None,
                shortcut: false,
            },
            Ghost {
                at: (175, 40),
                name: "Box",
                icon: None,
                shortcut: false,
            },
        ];
        let (img, grab) = drag_image(&theme, &t, &ghosts, DESKTOP, (150, 50)).expect("an image");
        assert_eq!((img.width, img.height), (150, 75));
        assert_eq!(grab, (50, 10));
        assert!(holds(&img, &ghost_cell(&theme, &t, "a.txt"), (0, 0)));
        assert!(holds(&img, &ghost_cell(&theme, &t, "Box"), (75, 0)));
    }

    #[test]
    fn a_drag_image_of_three_keeps_the_gaps_between_them() {
        let (theme, t) = (Theme::default(), text());
        let ghosts: Vec<Ghost> = ["one", "two", "three"]
            .iter()
            .enumerate()
            .map(|(i, name)| Ghost {
                at: (8, 8 + 80 * i as i32),
                name,
                icon: None,
                shortcut: false,
            })
            .collect();
        let (img, grab) = drag_image(&theme, &t, &ghosts, DESKTOP, (40, 120)).expect("an image");
        assert_eq!((img.width, img.height), (75, 235));
        assert_eq!(grab, (32, 112));
        for (i, name) in ["one", "two", "three"].iter().enumerate() {
            assert!(holds(
                &img,
                &ghost_cell(&theme, &t, name),
                (0, 80 * i as i32)
            ));
        }
        assert!((0..75).all(|x| img.get(x, 77) == 0));
    }

    /// At a larger text size each ghost is the grown cell, its label wrapped in the grown width.
    #[test]
    fn a_grown_drag_image_draws_each_ghost_in_the_grown_cell() {
        const EXPECTED: [(u32, (i32, i32)); 2] = [(150, (116, 95)), (200, (150, 112))];
        let name = "Holiday photos from the hills";
        for (pct, size) in EXPECTED {
            let theme = Theme::default().with_text_size(pct);
            let t = TextRenderer::load(&theme.fonts);
            let cell = Cell::grown(theme.text);
            assert_eq!((cell.w, cell.h), size, "{pct}%");
            let ghosts = [
                Ghost {
                    at: (0, 0),
                    name,
                    icon: None,
                    shortcut: false,
                },
                Ghost {
                    at: (0, cell.h),
                    name: "Box",
                    icon: None,
                    shortcut: true,
                },
            ];
            let pointer = (cell.w / 2, cell.h);
            let (img, grab) = drag_image(&theme, &t, &ghosts, cell, pointer).expect("an image");
            assert_eq!((img.width as i32, img.height as i32), (cell.w, 2 * cell.h));
            assert_eq!(grab, pointer);
            for (k, g) in ghosts.iter().enumerate() {
                let mut one = Canvas::new(cell.w as u32, cell.h as u32);
                draw_cell_at(
                    &mut one,
                    (0, 0),
                    cell,
                    &theme,
                    &t,
                    g.name,
                    None,
                    g.shortcut,
                    None,
                    None,
                );
                one.fade(1, 2);
                assert!(holds(&img, &one, (0, k as i32 * cell.h)), "{pct}%: {k}");
            }
            let label = label_layout(&t, name, theme.fonts.size_px, 1, cell.w);
            assert!(
                label.lines.iter().all(|l| l.width <= cell.w - 4),
                "{pct}%: wrapped in the grown width"
            );
            assert!(
                label.rect.width > CELL,
                "{pct}%: wider than a desktop cell's label"
            );
        }
    }

    /// A 6x6 grid of grown cells dragged from the middle of cell (2, 2).
    #[test]
    fn the_drag_image_square_grows_with_the_text_size_and_keeps_whole_cells() {
        type Px = (i32, i32);
        const EXPECTED: [(u32, Px, Px); 3] = [
            (100, (256, 256), (128, 128)),
            (150, (348, 285), (174, 142)),
            (200, (450, 336), (225, 168)),
        ];
        let name = "Holiday photos from the hills";
        for (pct, size, grab_at) in EXPECTED {
            let theme = Theme::default().with_text_size(pct);
            let t = TextRenderer::load(&theme.fonts);
            let cell = Cell::grown(theme.text);
            let ghosts: Vec<Ghost> = (0..36)
                .map(|i| Ghost {
                    at: (i % 6 * cell.w, i / 6 * cell.h),
                    name,
                    icon: None,
                    shortcut: false,
                })
                .collect();
            let pointer = (cell.w * 5 / 2, cell.h * 5 / 2);
            let (img, grab) = drag_image(&theme, &t, &ghosts, cell, pointer).expect("an image");
            assert_eq!((img.width as i32, img.height as i32), size, "{pct}%");
            assert_eq!(grab, grab_at, "{pct}%");
            if pct == 100 {
                continue;
            }
            let mut one = Canvas::new(cell.w as u32, cell.h as u32);
            draw_cell_at(
                &mut one,
                (0, 0),
                cell,
                &theme,
                &t,
                name,
                None,
                false,
                None,
                None,
            );
            one.fade(1, 2);
            let origin = (pointer.0 - grab.0, pointer.1 - grab.1);
            assert_eq!(origin, (cell.w, cell.h), "{pct}%: starts at cell (1, 1)");
            for (col, row) in (0..3).flat_map(|c| (0..3).map(move |r| (c, r))) {
                assert!(
                    holds(&img, &one, (col * cell.w, row * cell.h)),
                    "{pct}%: cell ({col}, {row}) whole"
                );
            }
        }
    }

    #[test]
    fn a_grown_drag_image_with_no_whole_cell_keeps_the_cut() {
        let theme = Theme::default().with_text_size(200);
        let t = TextRenderer::load(&theme.fonts);
        let cell = Cell::grown(theme.text);
        let ghosts = [Ghost {
            at: (0, 0),
            name: "a.txt",
            icon: None,
            shortcut: false,
        }];
        let (img, grab) =
            drag_image(&theme, &t, &ghosts, cell, (cell.w + 200, 10)).expect("an image");
        assert_eq!((img.width as i32, img.height as i32), (cell.w - 94, cell.h));
        assert_eq!(grab, (256, 10));
    }

    #[test]
    fn a_drag_image_is_cut_to_256_px_around_the_pointer() {
        let (theme, t) = (Theme::default(), text());
        let ghosts: Vec<Ghost> = (0..8)
            .map(|i| Ghost {
                at: (75 * i, 0),
                name: "x",
                icon: None,
                shortcut: false,
            })
            .collect();
        let (img, grab) = drag_image(&theme, &t, &ghosts, DESKTOP, (40, 40)).expect("an image");
        assert_eq!((img.width, img.height), (168, 75));
        assert_eq!(grab, (40, 40));
        let (img, grab) = drag_image(&theme, &t, &ghosts, DESKTOP, (300, 40)).expect("an image");
        assert_eq!((img.width, img.height), (256, 75));
        assert_eq!(grab, (128, 40));
        assert!(drag_image(&theme, &t, &ghosts[..1], DESKTOP, (1000, 1000)).is_none());
        assert!(drag_image(&theme, &t, &[], DESKTOP, (0, 0)).is_none());
    }
}
