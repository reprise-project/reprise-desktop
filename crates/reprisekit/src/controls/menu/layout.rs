use super::MenuItem;
use crate::metrics::StartMenuMetrics;
use crate::raster::{Insets, Rect};
use crate::text::TextRenderer;
use crate::theme::{Metrics, Theme, line_growth};

/// Inset of a menu's rows from its edges: the border plus its pad. The
/// Start menu's comes from its look ([`StartMenuMetrics`]).
pub(super) const BORDER: i32 = 3;
pub(super) const INSET: Insets = Insets::new(BORDER, BORDER, BORDER, BORDER);
pub(super) const SEPARATOR_H: i32 = 8;
pub(super) const ICON_X: i32 = 0;
pub(super) const TEXT_X: i32 = 24;
const ROOT_ICON_X: i32 = 6;
pub(super) const ROOT_TEXT_X: i32 = 42;
pub const CHECK_X: i32 = 4;
/// How far below the centred position the check glyph sits: its top
/// row is 6 px down a 17 px row.
pub(super) const CHECK_DY: i32 = 1;
pub const BULLET_X: i32 = CHECK_X + 1;
pub const ARROW_RIGHT_INSET: i32 = 9;
pub(super) const ARROW_SPACE: i32 = ARROW_RIGHT_INSET + 4;
/// Space between the longest label's end and the menu's right edge on a
/// Start submenu (arrow column and margins), less the border `fit` adds. A
/// context menu's is its style's `right_pad`.
pub(super) const RIGHT_PAD: i32 = 20;
pub(super) const ROOT_RIGHT_PAD: i32 = 26;
pub(super) const BANNER_PAD: i32 = 4;
const SHORTCUT_GAP: i32 = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextBitmaps {
    None,
    Glyphs,
    Icons,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuStyle {
    pub width: i32,
    pub row_height: i32,
    pub band: i32,
    pub text_x: i32,
    pub icon_x: i32,
    pub separator_h: i32,
    pub right_pad: i32,
    /// How far in from the menu's edges its rows sit, border included
    /// (logical px). The band starts `inset.left` in on a Classic menu,
    /// right inside the line on a flat one.
    pub inset: Insets,
    pub separator_x: i32,
    pub lit_arrow_menu_text: bool,
    pub shortcut_pad: i32,
    pub gutter: i32,
}

impl MenuStyle {
    pub fn submenu(theme: &Theme) -> MenuStyle {
        let sm = theme.look.controls().start_menu;
        MenuStyle {
            inset: sm.sub,
            separator_x: sm.separator_x,
            lit_arrow_menu_text: sm.lit_arrow_menu_text,
            ..Self::compact(&theme.metrics)
        }
    }

    pub(super) fn compact(m: &Metrics) -> MenuStyle {
        MenuStyle {
            width: m.menu_width,
            row_height: m.menu_item_height,
            band: 0,
            text_x: TEXT_X,
            icon_x: ICON_X,
            separator_h: SEPARATOR_H,
            right_pad: RIGHT_PAD,
            inset: INSET,
            separator_x: StartMenuMetrics::CLASSIC.separator_x,
            lit_arrow_menu_text: false,
            shortcut_pad: RIGHT_PAD,
            gutter: 0,
        }
    }

    /// A look with a [`MenuLook`](crate::style::MenuLook) lays every
    /// context menu out its own way, bitmaps or not: its rows, its
    /// separators, its gutter and the label past it.
    pub fn context(theme: &Theme, bitmaps: ContextBitmaps) -> MenuStyle {
        let m = &theme.metrics;
        let grow = line_growth(theme.fonts.size_px / theme.scale.max(1) as u32);
        if let Some(ml) = theme.look.style().and_then(|st| st.menu) {
            return MenuStyle {
                width: m.menu_width,
                row_height: ml.row + grow,
                band: 0,
                text_x: ml.text_x,
                icon_x: ml.icon_x,
                separator_h: ml.separator,
                right_pad: ml.right_pad,
                inset: INSET,
                separator_x: 0,
                lit_arrow_menu_text: false,
                shortcut_pad: ml.shortcut_end,
                gutter: ml.gutter,
            };
        }
        let (row_height, icon_x, text_x, right_pad) = match bitmaps {
            ContextBitmaps::None => (17, 6, 17, 21),
            ContextBitmaps::Glyphs => (19, 2, 22, 20),
            ContextBitmaps::Icons => (19, 6, 28, 31),
        };
        MenuStyle {
            width: m.menu_width,
            row_height: row_height + grow,
            band: 0,
            text_x,
            icon_x,
            separator_h: 9,
            right_pad,
            inset: INSET,
            separator_x: StartMenuMetrics::CLASSIC.separator_x,
            lit_arrow_menu_text: false,
            shortcut_pad: right_pad,
            gutter: 0,
        }
    }

    pub fn context_for(theme: &Theme, items: &[MenuItem]) -> MenuStyle {
        let any = |f: fn(&MenuItem) -> bool| items.iter().any(f);
        let bitmaps = if any(|it| matches!(it, MenuItem::Entry { icon: Some(_), .. })) {
            ContextBitmaps::Icons
        } else if any(|it| matches!(it, MenuItem::Entry { glyph: Some(_), .. })) {
            ContextBitmaps::Glyphs
        } else {
            ContextBitmaps::None
        };
        Self::context(theme, bitmaps)
    }

    pub fn root(theme: &Theme) -> MenuStyle {
        let m = &theme.metrics;
        let sm = theme.look.controls().start_menu;
        MenuStyle {
            width: m.startmenu_width,
            row_height: m.startmenu_item_height,
            band: m.startmenu_band_width,
            text_x: ROOT_TEXT_X,
            icon_x: ROOT_ICON_X,
            separator_h: SEPARATOR_H,
            right_pad: ROOT_RIGHT_PAD,
            inset: sm.root,
            separator_x: sm.separator_x,
            lit_arrow_menu_text: sm.lit_arrow_menu_text,
            shortcut_pad: ROOT_RIGHT_PAD,
            gutter: 0,
        }
    }

    /// Icon edge length that fits this style's rows (logical px): the
    /// classic 16 px for compact rows, the row height minus a margin for the
    /// root menu's tall ones.
    pub fn icon_px(&self) -> i32 {
        if self.band > 0 {
            (self.row_height - 8).max(crate::icon::ICON_SIZE)
        } else {
            crate::icon::ICON_SIZE
        }
    }

    /// Widens to fit the longest label in `items` up to `max_width` (longer ones are ellipsized); `width` is a minimum.
    pub fn fit(
        self,
        text: &TextRenderer,
        items: &[MenuItem],
        size_px: u32,
        max_width: i32,
    ) -> MenuStyle {
        let longest = longest_row(text, items, size_px, self.right_pad, self.shortcut_pad);
        let needed = self.band + self.inset.left + self.text_x + longest + self.inset.right;
        MenuStyle {
            width: needed.min(max_width).max(self.width),
            ..self
        }
    }
}

/// The widest row of `items` from its text's start to the menu's inner right edge.
fn longest_row(
    text: &TextRenderer,
    items: &[MenuItem],
    size_px: u32,
    right_pad: i32,
    shortcut_pad: i32,
) -> i32 {
    items
        .iter()
        .map(|it| match it {
            MenuItem::Entry {
                label,
                bold,
                shortcut,
                ..
            } => {
                let sc = shortcut
                    .as_deref()
                    .map(|sc| text.measure(sc, false, size_px) + SHORTCUT_GAP + shortcut_pad)
                    .unwrap_or(right_pad);
                text.measure(label, *bold, size_px) + sc
            }
            MenuItem::Separator => right_pad,
        })
        .max()
        .unwrap_or(right_pad)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuLayout {
    pub width: i32,
    pub height: i32,
    pub rows: Vec<Rect>,
    /// Parallel to `rows`: true for an `Entry` row, false for a `Separator`.
    pub selectable: Vec<bool>,
    pub columns: usize,
    pub column_width: i32,
    pub band: i32,
    pub text_x: i32,
    pub icon_x: i32,
    pub inset: Insets,
    pub separator_x: i32,
    pub lit_arrow_menu_text: bool,
    pub gutter: i32,
    /// The heights `compute_styled` gives an `Entry` row and a `Separator`, for [`relist`].
    pub row_height: i32,
    pub separator_h: i32,
    /// The style's paddings past a label, for [`MenuLayout::fit_column`].
    pub right_pad: i32,
    pub shortcut_pad: i32,
}

impl MenuLayout {
    #[cfg(test)]
    pub fn compute(m: &Metrics, items: &[MenuItem]) -> Self {
        Self::compute_columns(m, items, usize::MAX)
    }

    #[cfg(test)]
    pub fn compute_columns(m: &Metrics, items: &[MenuItem], max_rows: usize) -> Self {
        Self::compute_styled(&MenuStyle::compact(m), items, max_rows)
    }

    pub fn compute_styled(style: &MenuStyle, items: &[MenuItem], max_rows: usize) -> Self {
        let rows_per_col = max_rows.max(1);
        let column_width = style.width - style.band;
        let inset = style.inset;
        let inner_w = column_width - inset.left - inset.right;
        let mut rows = Vec::with_capacity(items.len());
        let mut selectable = Vec::with_capacity(items.len());
        let mut height = inset.top + inset.bottom;
        let mut columns = 0usize;
        for chunk in items.chunks(rows_per_col) {
            let x = style.band + inset.left + columns as i32 * column_width;
            columns += 1;
            let mut y = inset.top;
            for it in chunk {
                let (h, sel) = match it {
                    MenuItem::Entry { .. } => (style.row_height, true),
                    MenuItem::Separator => (style.separator_h, false),
                };
                rows.push(Rect::new(x, y, inner_w, h));
                selectable.push(sel);
                y += h;
            }
            height = height.max(y + inset.bottom);
        }
        let columns = columns.max(1);
        MenuLayout {
            width: style.band + columns as i32 * column_width,
            height,
            rows,
            selectable,
            columns,
            column_width,
            band: style.band,
            text_x: style.text_x,
            icon_x: style.icon_x,
            inset,
            separator_x: style.separator_x,
            lit_arrow_menu_text: style.lit_arrow_menu_text,
            gutter: style.gutter,
            row_height: style.row_height,
            separator_h: style.separator_h,
            right_pad: style.right_pad,
            shortcut_pad: style.shortcut_pad,
        }
    }

    /// The column width `items` need, as [`MenuStyle::fit`] widens it, up to a menu `max_width`
    /// wide; never narrower than the columns now. For [`relist`].
    pub fn fit_column(
        &self,
        text: &TextRenderer,
        items: &[MenuItem],
        size_px: u32,
        max_width: i32,
    ) -> i32 {
        let longest = longest_row(text, items, size_px, self.right_pad, self.shortcut_pad);
        let needed = self.band + self.inset.left + self.text_x + longest + self.inset.right;
        (needed.min(max_width) - self.band).max(self.column_width)
    }

    /// Index of the entry under (x, y); separators and borders return None.
    /// Rows are excluded by kind (`selectable`), not by height, so a
    /// configured `menu_item_height` equal to the separator height still
    /// hit-tests correctly.
    pub fn hit(&self, x: i32, y: i32) -> Option<usize> {
        self.rows
            .iter()
            .zip(&self.selectable)
            .position(|(r, sel)| *sel && r.contains(x, y))
    }

    pub fn scaled(&self, n: i32) -> MenuLayout {
        let n = n.max(1);
        MenuLayout {
            width: self.width * n,
            height: self.height * n,
            rows: self.rows.iter().map(|r| r.scaled(n)).collect(),
            selectable: self.selectable.clone(),
            columns: self.columns,
            column_width: self.column_width * n,
            band: self.band * n,
            text_x: self.text_x * n,
            icon_x: self.icon_x * n,
            inset: self.inset.scaled(n),
            separator_x: self.separator_x * n,
            lit_arrow_menu_text: self.lit_arrow_menu_text,
            gutter: self.gutter * n,
            row_height: self.row_height * n,
            separator_h: self.separator_h * n,
            right_pad: self.right_pad * n,
            shortcut_pad: self.shortcut_pad * n,
        }
    }
}

/// Relists an open menu as `new_items` in columns `column_width` wide (normally
/// [`MenuLayout::fit_column`]): a row whose key `old_keys` holds keeps its item (and icon); rows
/// per column stay; the hover follows its entry (`selection::follow`).
pub fn relist<K: PartialEq>(
    items: &mut Vec<MenuItem>,
    layout: &mut MenuLayout,
    hovered: &mut Option<usize>,
    old_keys: &[K],
    new_keys: &[K],
    new_items: Vec<MenuItem>,
    column_width: i32,
) {
    let mut old: Vec<Option<MenuItem>> = std::mem::take(items).into_iter().map(Some).collect();
    *items = new_keys
        .iter()
        .zip(new_items)
        .map(|(k, new)| {
            old_keys
                .iter()
                .position(|o| o == k)
                .and_then(|i| old.get_mut(i)?.take())
                .unwrap_or(new)
        })
        .collect();
    *hovered = hovered
        .and_then(|h| crate::selection::follow(old_keys, new_keys, h, |j| items[j].selectable()));
    let rows_per_col = if layout.columns > 1 {
        layout
            .rows
            .iter()
            .take_while(|r| r.x == layout.rows[0].x)
            .count()
            .max(1)
    } else {
        usize::MAX
    };
    layout.column_width = column_width;
    let inner_w = layout.column_width - layout.inset.left - layout.inset.right;
    let inset = layout.inset;
    let mut rows = Vec::with_capacity(items.len());
    let mut height = inset.top + inset.bottom;
    let mut columns = 0;
    for chunk in items.chunks(rows_per_col) {
        let x = layout.band + inset.left + columns * layout.column_width;
        columns += 1;
        let mut y = inset.top;
        for it in chunk {
            let h = match it {
                MenuItem::Entry { .. } => layout.row_height,
                MenuItem::Separator => layout.separator_h,
            };
            rows.push(Rect::new(x, y, inner_w, h));
            y += h;
        }
        height = height.max(y + inset.bottom);
    }
    layout.columns = (columns as usize).max(1);
    layout.width = layout.band + layout.columns as i32 * layout.column_width;
    layout.height = height;
    layout.rows = rows;
    layout.selectable = items
        .iter()
        .map(|it| matches!(it, MenuItem::Entry { .. }))
        .collect();
}

pub fn submenu_max_rows(m: &Metrics, usable_height: i32) -> usize {
    (((usable_height - 2 * BORDER) / m.menu_item_height.max(1)).max(1)) as usize
}

pub fn place_popup(at: (i32, i32), size: (i32, i32), usable: Rect) -> (i32, i32) {
    let (w, h) = size;
    let (mut x, mut y) = at;
    if x + w > usable.right() {
        x = at.0 - w;
    }
    if y + h > usable.bottom() {
        y = at.1 - h;
    }
    (
        x.min(usable.right() - w).max(usable.x),
        y.min(usable.bottom() - h).max(usable.y),
    )
}

pub fn place_submenu(parent: Rect, row: Rect, size: (i32, i32), usable: Rect) -> (i32, i32) {
    place_submenu_inset(parent, row, size, usable, BORDER, INSET)
}

/// [`place_submenu`] for menus with their own borders: the parent's rows
/// end `parent_right` short of its right edge, and the child's rows sit
/// `child` in from its edges (their layouts' `inset`).
pub fn place_submenu_inset(
    parent: Rect,
    row: Rect,
    size: (i32, i32),
    usable: Rect,
    parent_right: i32,
    child: Insets,
) -> (i32, i32) {
    let (w, h) = size;
    // The submenu overlaps the parent by both borders, so its first row
    // column starts right after the parent's last one: Classic's child's
    // left border starts 6 px short of the parent's right edge.
    let overlap = parent_right + child.left;
    let mut x = parent.right() - overlap;
    if x + w > usable.right() {
        x = parent.x - w + overlap;
        // Wider than the usable area (or the flip still runs past its left
        // edge): pin to the left rather than spilling off-screen.
        x = x.max(usable.x);
    }
    let mut y = row.y - child.top;
    if y + h > usable.bottom() {
        y = usable.bottom() - h;
    }
    y = y.max(usable.y);
    (x, y)
}
