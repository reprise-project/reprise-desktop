use super::MenuItem;
use crate::icon::ICON_SIZE;
use crate::keycap;
use crate::metrics::StartMenuMetrics;
use crate::raster::{Insets, Rect};
use crate::text::TextRenderer;
use crate::theme::{Metrics, TextUnits, Theme, line_growth};

/// Inset of a menu's rows from its edges: the border plus its pad. The
/// Start menu's comes from its look ([`StartMenuMetrics`]).
pub(super) const BORDER: i32 = 3;
pub(super) const INSET: Insets = Insets::new(BORDER, BORDER, BORDER, BORDER);
pub(super) const SEPARATOR_H: i32 = 8;
pub(super) const ICON_X: i32 = 0;
pub(super) const TEXT_X: i32 = 24;
const ROOT_ICON_X: i32 = 6;
/// From a Start menu row's icon to its label; the icon grows with the row.
const ROOT_LABEL_GAP: i32 = 12;
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
/// The banner's start up from the band's bottom, and its font's px over the UI font's, at 100%
/// text: both grow with the text size, as the band does.
pub(super) const BANNER_PAD: i32 = 4;
pub(super) const BANNER_OVER: i32 = 3;
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
    /// The rows' icon edge (logical px): [`MenuStyle::icon_px`].
    pub icon: i32,
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
    /// The least height the menu takes whatever its rows (logical px): see [`MenuStyle::hold_banner`].
    pub min_height: i32,
    /// The output scale the menu draws at: [`MenuStyle::fit`] measures labels there.
    pub scale: i32,
}

impl MenuStyle {
    /// A Start submenu: its icon column grows with the text size ([`MenuStyle::grown`]).
    pub fn submenu(theme: &Theme) -> MenuStyle {
        let sm = theme.look.controls_at_100().start_menu;
        MenuStyle {
            inset: sm.sub,
            separator_x: sm.separator_x,
            lit_arrow_menu_text: sm.lit_arrow_menu_text,
            ..Self::compact(&theme.metrics)
        }
        .grown(theme.text, true, 0, ARROW_RIGHT_INSET)
    }

    pub(super) fn compact(m: &Metrics) -> MenuStyle {
        MenuStyle {
            width: m.menu_width,
            row_height: m.menu_item_height,
            band: 0,
            text_x: TEXT_X,
            icon_x: ICON_X,
            icon: ICON_SIZE,
            separator_h: SEPARATOR_H,
            right_pad: RIGHT_PAD,
            inset: INSET,
            separator_x: StartMenuMetrics::CLASSIC.separator_x,
            lit_arrow_menu_text: false,
            shortcut_pad: RIGHT_PAD,
            gutter: 0,
            min_height: 0,
            scale: 1,
        }
    }

    /// A look with a [`MenuLook`](crate::style::MenuLook) lays every
    /// context menu out its own way: its rows, its separators, its gutter and
    /// the label past it. With `Icons` the icon column grows with the text
    /// size ([`MenuStyle::grown`]).
    pub fn context(theme: &Theme, bitmaps: ContextBitmaps) -> MenuStyle {
        let style = Self::context_at_100(theme, bitmaps);
        // A look's own menu checks in a square the row's height inside its gutter.
        let (column, arrow) = match theme.look.style().and_then(|st| st.menu) {
            Some(ml) => (ml.gutter, ml.arrow_inset),
            None => (style.text_x, ARROW_RIGHT_INSET),
        };
        style.grown(theme.text, bitmaps == ContextBitmaps::Icons, column, arrow)
    }

    fn context_at_100(theme: &Theme, bitmaps: ContextBitmaps) -> MenuStyle {
        let m = &theme.metrics;
        let grow = line_growth(theme.fonts.size_px / theme.scale.max(1) as u32);
        if let Some(ml) = theme.look.style().and_then(|st| st.menu) {
            return MenuStyle {
                width: m.menu_width,
                row_height: ml.row + grow,
                band: 0,
                text_x: ml.text_x,
                icon_x: ml.icon_x,
                icon: ICON_SIZE,
                separator_h: ml.separator,
                right_pad: ml.right_pad,
                inset: ml.inset,
                separator_x: 0,
                lit_arrow_menu_text: false,
                shortcut_pad: ml.shortcut_end,
                gutter: ml.gutter,
                min_height: 0,
                scale: 1,
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
            icon: ICON_SIZE,
            separator_h: 9,
            right_pad,
            inset: INSET,
            separator_x: StartMenuMetrics::CLASSIC.separator_x,
            lit_arrow_menu_text: false,
            shortcut_pad: right_pad,
            gutter: 0,
            min_height: 0,
            scale: 1,
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

    /// The Start menu: its band, its rows' icons and the labels past them grow with the text size.
    pub fn root(theme: &Theme) -> MenuStyle {
        let m = &theme.metrics;
        let sm = theme.look.controls_at_100().start_menu;
        let icon = (m.startmenu_item_height - 8).max(ICON_SIZE);
        MenuStyle {
            width: m.startmenu_width,
            row_height: m.startmenu_item_height,
            band: theme.text.len(m.startmenu_band_width),
            text_x: ROOT_ICON_X + icon + ROOT_LABEL_GAP,
            icon_x: ROOT_ICON_X,
            icon,
            separator_h: SEPARATOR_H,
            right_pad: ROOT_RIGHT_PAD,
            inset: sm.root,
            separator_x: sm.separator_x,
            lit_arrow_menu_text: sm.lit_arrow_menu_text,
            shortcut_pad: ROOT_RIGHT_PAD,
            gutter: 0,
            min_height: 0,
            scale: 1,
        }
    }

    /// This style for text `u`. Its check marks and submenu arrows grow by [`TextUnits::small`].
    /// With `icons` its 16 px icon column, which holds the marks, grows as [`TextUnits::icon`]
    /// does; without, the `column` left of the label grows with the marks. The label, and a
    /// gutter, move right by that growth, and the right pad widens where it must to keep the
    /// label as clear of the grown arrow, inset `arrow` px at 100%, as a Classic row's is.
    fn grown(self, u: TextUnits, icons: bool, column: i32, arrow: i32) -> MenuStyle {
        let k = u.small();
        let (icon, more) = if icons {
            (u.icon(ICON_SIZE), u.icon(ICON_SIZE) - ICON_SIZE)
        } else {
            (ICON_SIZE, (k - 1) * column)
        };
        MenuStyle {
            icon,
            text_x: self.text_x + more,
            gutter: if self.gutter > 0 {
                self.gutter + more
            } else {
                0
            },
            right_pad: if k > 1 {
                self.right_pad
                    .max(k * (arrow + ARROW_SPACE - ARROW_RIGHT_INSET))
            } else {
                self.right_pad
            },
            ..self
        }
    }

    /// This style no shorter than its band's `banner` with a [`BANNER_PAD`] at both ends: a Start
    /// menu with few rows or none still shows its whole banner. `theme` and `text` are the
    /// output's scale ones the band draws with, whose glyphs may run longer than twice scale 1's.
    pub fn hold_banner(self, theme: &Theme, text: &TextRenderer, banner: &str) -> MenuStyle {
        let s = theme.scale.max(1);
        let pad = theme.text.len(BANNER_PAD);
        let device: i32 = super::draw::banner_parts(banner)
            .map(|(part, bold)| text.measure(&part, bold, super::draw::banner_px(theme)))
            .sum();
        let long = (device + s - 1) / s;
        MenuStyle {
            min_height: self.inset.top + pad + long + pad + self.inset.bottom,
            ..self
        }
    }

    /// Icon edge length that fits this style's rows (logical px): 16 px at 100% text growing
    /// with the text size, the row height minus a margin for the root menu's tall rows. Hosts
    /// load the rows' icons at it.
    pub fn icon_px(&self) -> i32 {
        self.icon
    }

    /// This style for a menu drawn at output scale `scale`.
    pub fn at_scale(self, scale: i32) -> MenuStyle {
        MenuStyle {
            scale: scale.max(1),
            ..self
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
        let longest = longest_row(
            text,
            items,
            size_px,
            self.scale,
            self.right_pad,
            self.shortcut_pad,
        );
        let needed = self.band + self.inset.left + self.text_x + longest + self.inset.right;
        MenuStyle {
            width: needed.min(max_width).max(self.width),
            ..self
        }
    }

    /// [`MenuStyle::layout_within`] `usable` at the theme's text size.
    pub fn layout_in(
        self,
        theme: &Theme,
        text: &TextRenderer,
        items: &[MenuItem],
        usable: Rect,
    ) -> MenuLayout {
        let room = (usable.width, usable.height);
        self.layout_within(text, items, theme.fonts.size_px, room)
    }

    /// `items` in a menu at most `room` (width, height) big: too tall, it wraps into columns,
    /// each fitted ([`MenuStyle::fit`]) to its share of the width. When a label would not fit
    /// its column's share, the menu stays one column `height` tall that scrolls
    /// ([`MenuLayout::scroll`]).
    pub fn layout_within(
        self,
        text: &TextRenderer,
        items: &[MenuItem],
        size_px: u32,
        (width, height): (i32, i32),
    ) -> MenuLayout {
        let rows = self.rows_in(height);
        let columns = items.len().div_ceil(rows).max(1) as i32;
        let share = (width - self.band) / columns + self.band;
        let style = self.fit(text, items, size_px, share);
        let whole = self.fit(text, items, size_px, width);
        if columns > 1 && style.width < whole.width {
            return MenuLayout::compute_styled(&whole, items, usize::MAX).scrolled_within(height);
        }
        MenuLayout::compute_styled(&style, items, rows)
    }

    /// The rows a column `height` tall holds, at least one: as many of the tallest kind of row
    /// as fit inside the insets.
    pub fn rows_in(&self, height: i32) -> usize {
        let tallest = self.row_height.max(self.separator_h).max(1);
        ((height - self.inset.top - self.inset.bottom) / tallest).max(1) as usize
    }
}

/// The widest row of `items` from its text's start to the menu's inner right edge, logical px.
/// Text is measured as it draws at `scale`, whose hinted glyphs may run longer than `scale`
/// times scale 1's, and rounded up.
fn longest_row(
    text: &TextRenderer,
    items: &[MenuItem],
    size_px: u32,
    scale: i32,
    right_pad: i32,
    shortcut_pad: i32,
) -> i32 {
    let device = text.for_output(scale);
    let measure = |t: &str, bold: bool| {
        let w = device.measure(t, bold, size_px * scale as u32);
        (w + scale - 1) / scale
    };
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
                    .map(|sc| {
                        measure(&keycap::shown_chord(sc), false) + SHORTCUT_GAP + shortcut_pad
                    })
                    .unwrap_or(right_pad);
                measure(label, *bold) + sc
            }
            MenuItem::Heading(title) => measure(title, true) + right_pad,
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
    /// Parallel to `rows`: true for an `Entry` row, false for a `Separator` or `Heading`.
    pub selectable: Vec<bool>,
    pub columns: usize,
    pub column_width: i32,
    /// The most rows a column takes before the next starts; `usize::MAX` for one column
    /// whatever its length.
    pub rows_per_column: usize,
    pub band: i32,
    pub text_x: i32,
    pub icon_x: i32,
    pub icon: i32,
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
    pub min_height: i32,
    /// The output scale its labels were fitted for ([`MenuStyle::scale`]).
    pub scale: i32,
    /// Set on a menu too tall for its room: one column between two scroll arrows. `rows` hold
    /// where each row stands now; only the rows wholly between the arrows show
    /// ([`MenuLayout::shown`]).
    pub scroll: Option<MenuScroll>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuScroll {
    /// The row at the top, under the up arrow.
    pub first: usize,
    /// Each arrow band's height.
    pub arrow: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollArrow {
    Up,
    Down,
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
                    MenuItem::Heading(_) => (style.row_height, false),
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
            height: height.max(style.min_height),
            rows,
            selectable,
            columns,
            column_width,
            rows_per_column: rows_per_col,
            band: style.band,
            text_x: style.text_x,
            icon_x: style.icon_x,
            icon: style.icon,
            inset,
            separator_x: style.separator_x,
            lit_arrow_menu_text: style.lit_arrow_menu_text,
            gutter: style.gutter,
            row_height: style.row_height,
            separator_h: style.separator_h,
            right_pad: style.right_pad,
            shortcut_pad: style.shortcut_pad,
            min_height: style.min_height,
            scale: style.scale,
            scroll: None,
        }
    }

    /// This one-column layout cut to `height`, between scroll arrows half a row tall, when its
    /// rows need more.
    fn scrolled_within(mut self, height: i32) -> MenuLayout {
        if self.height <= height {
            return self;
        }
        let arrow = (self.row_height / 2).max(1);
        self.height = height;
        for r in &mut self.rows {
            r.y += arrow;
        }
        self.scroll = Some(MenuScroll { first: 0, arrow });
        self
    }

    /// The span between the scroll arrows, top and bottom; None when the menu does not scroll.
    fn view(&self) -> Option<(i32, i32)> {
        let sc = self.scroll?;
        let top = self.inset.top + sc.arrow;
        Some((top, self.height - self.inset.bottom - sc.arrow))
    }

    /// Whether row `row` stands wholly in view: every row of a menu that does not scroll.
    pub fn shown(&self, row: usize) -> bool {
        match (self.view(), self.rows.get(row)) {
            (Some((top, bottom)), Some(r)) => r.y >= top && r.bottom() <= bottom,
            (None, r) => r.is_some(),
            (_, None) => false,
        }
    }

    /// The last row that can stand at the top: the one past which the rest fit in view.
    fn last_first(&self) -> usize {
        let Some((top, bottom)) = self.view() else {
            return 0;
        };
        let Some(end) = self.rows.last().map(Rect::bottom) else {
            return 0;
        };
        (0..self.rows.len())
            .find(|&i| end - self.rows[i].y <= bottom - top)
            .unwrap_or(0)
    }

    /// Scrolls so row `first` (held to the rows that can) stands at the top; true when it moved.
    pub fn scroll_to(&mut self, first: usize) -> bool {
        let Some(sc) = self.scroll else {
            return false;
        };
        let first = first.min(self.last_first());
        if first == sc.first {
            return false;
        }
        let dy = self.rows[sc.first].y - self.rows[first].y;
        for r in &mut self.rows {
            r.y += dy;
        }
        self.scroll = Some(MenuScroll { first, ..sc });
        true
    }

    /// Scrolls `rows` rows down (up when negative); true when it moved.
    pub fn scroll_by(&mut self, rows: i32) -> bool {
        let first = self.scroll.map_or(0, |sc| sc.first as i32);
        self.scroll_to((first + rows).max(0) as usize)
    }

    /// Scrolls as little as brings row `row` wholly into view; true when it moved.
    pub fn scroll_to_show(&mut self, row: usize) -> bool {
        let (Some(sc), Some((top, bottom))) = (self.scroll, self.view()) else {
            return false;
        };
        if row >= self.rows.len() || self.shown(row) {
            return false;
        }
        if row < sc.first {
            return self.scroll_to(row);
        }
        let end = self.rows[row].bottom();
        let first = (sc.first..=row)
            .find(|&i| end - self.rows[i].y <= bottom - top)
            .unwrap_or(row);
        self.scroll_to(first)
    }

    /// Whether the menu can scroll on toward `arrow`.
    pub fn can_scroll(&self, arrow: ScrollArrow) -> bool {
        self.scroll.is_some_and(|sc| match arrow {
            ScrollArrow::Up => sc.first > 0,
            ScrollArrow::Down => sc.first < self.last_first(),
        })
    }

    /// The band `arrow` is drawn in, inside the border.
    pub fn arrow_rect(&self, arrow: ScrollArrow) -> Option<Rect> {
        let sc = self.scroll?;
        let ins = self.inset;
        let x = self.band + ins.left;
        let w = self.width - x - ins.right;
        let y = match arrow {
            ScrollArrow::Up => ins.top,
            ScrollArrow::Down => self.height - ins.bottom - sc.arrow,
        };
        Some(Rect::new(x, y, w, sc.arrow))
    }

    /// The scroll arrow under (x, y): its band and the border beyond it.
    pub fn arrow_at(&self, x: i32, y: i32) -> Option<ScrollArrow> {
        let (top, bottom) = self.view()?;
        if x < 0 || x >= self.width {
            return None;
        }
        match y {
            y if (0..top).contains(&y) => Some(ScrollArrow::Up),
            y if (bottom..self.height).contains(&y) => Some(ScrollArrow::Down),
            _ => None,
        }
    }

    /// The column width `items` need, as [`MenuStyle::fit`] widens it, up to their columns'
    /// share of a menu `max_width` wide; never narrower than the columns now. For [`relist`].
    pub fn fit_column(
        &self,
        text: &TextRenderer,
        items: &[MenuItem],
        size_px: u32,
        max_width: i32,
    ) -> i32 {
        let columns = items.len().div_ceil(self.rows_per_column).max(1) as i32;
        let share = (max_width - self.band) / columns + self.band;
        self.style().fit(text, items, size_px, share).width - self.band
    }

    /// The column row `row` stands in.
    pub fn column_of(&self, row: usize) -> usize {
        self.rows.get(row).map_or(0, |r| {
            ((r.x - self.band) / self.column_width.max(1)) as usize
        })
    }

    /// The row a `stop` can land on in column `column` nearest level with row `row`: where Left
    /// and Right go across a menu in columns. `None` past the columns or with no stop there.
    pub fn level_in(
        &self,
        row: usize,
        column: usize,
        stop: impl Fn(usize) -> bool,
    ) -> Option<usize> {
        let mid = |r: &Rect| r.y + r.height / 2;
        let y = mid(self.rows.get(row)?);
        (0..self.rows.len())
            .filter(|&i| self.column_of(i) == column && stop(i))
            .min_by_key(|&i| (mid(&self.rows[i]) - y).abs())
    }

    /// The style `compute_styled` laid this out from, at the current column width.
    fn style(&self) -> MenuStyle {
        MenuStyle {
            width: self.band + self.column_width,
            row_height: self.row_height,
            band: self.band,
            text_x: self.text_x,
            icon_x: self.icon_x,
            icon: self.icon,
            separator_h: self.separator_h,
            right_pad: self.right_pad,
            inset: self.inset,
            separator_x: self.separator_x,
            lit_arrow_menu_text: self.lit_arrow_menu_text,
            shortcut_pad: self.shortcut_pad,
            gutter: self.gutter,
            min_height: self.min_height,
            scale: self.scale,
        }
    }

    /// Index of the entry under (x, y); separators and borders return None.
    /// Rows are excluded by kind (`selectable`), not by height, so a
    /// configured `menu_item_height` equal to the separator height still
    /// hit-tests correctly.
    pub fn hit(&self, x: i32, y: i32) -> Option<usize> {
        (0..self.rows.len())
            .find(|&i| self.selectable[i] && self.shown(i) && self.rows[i].contains(x, y))
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
            rows_per_column: self.rows_per_column,
            band: self.band * n,
            text_x: self.text_x * n,
            icon_x: self.icon_x * n,
            icon: self.icon * n,
            inset: self.inset.scaled(n),
            separator_x: self.separator_x * n,
            lit_arrow_menu_text: self.lit_arrow_menu_text,
            gutter: self.gutter * n,
            row_height: self.row_height * n,
            separator_h: self.separator_h * n,
            right_pad: self.right_pad * n,
            shortcut_pad: self.shortcut_pad * n,
            min_height: self.min_height * n,
            scale: self.scale,
            scroll: self.scroll.map(|sc| MenuScroll {
                arrow: sc.arrow * n,
                ..sc
            }),
        }
    }
}

/// Relists an open menu as `new_items` in columns `column_width` wide (normally
/// [`MenuLayout::fit_column`]): a row whose key `old_keys` holds keeps its item (and icon); the
/// rows a column holds stay, so a longer list wraps into another column; a scrolling menu keeps
/// its height and top row; the hover follows its entry (`selection::follow`).
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
    layout.column_width = column_width;
    let fresh = MenuLayout::compute_styled(&layout.style(), items, layout.rows_per_column);
    *layout = match layout.scroll {
        Some(sc) => {
            let mut l = fresh.scrolled_within(layout.height);
            l.scroll_to(sc.first);
            l
        }
        None => fresh,
    };
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

/// [`place_submenu_inset`] for a look whose submenus stand beside their parent, frame to frame,
/// the child's first row (`child` in from its top) level with `row`.
pub fn place_submenu_beside(
    parent: Rect,
    row: Rect,
    size: (i32, i32),
    usable: Rect,
    child: Insets,
) -> (i32, i32) {
    let child = Insets::new(0, child.top, 0, 0);
    place_submenu_inset(parent, row, size, usable, 0, child)
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
