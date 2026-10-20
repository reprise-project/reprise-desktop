//! Menu popups: layout, drawing and open-chain behaviour for context menus, submenus, tray menus and
//! the root Start menu. The host draws the returned canvas and feeds `hit` / `step_selection` / `enter_row`.

use std::borrow::Cow;

use crate::access::{self, Tree, accesskit};
use crate::metrics::StartMenuMetrics;
use crate::raster::{Canvas, EdgeStyle, Insets, Rect};
use crate::text::{Align, TextRenderer, split_mnemonic};
use crate::theme::{Metrics, Theme, line_growth};
use accesskit::{Node, NodeId, Role};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuItem {
    Entry {
        label: String,
        icon: Option<Canvas>,
        /// Drawn in `gray_text` and never hovered when false (tray menus).
        enabled: bool,
        checked: bool,
        radio: bool,
        submenu: bool,
        mnemonic: Option<usize>,
        shortcut: Option<String>,
        /// Bold label: the menu's default row (the window menu's Close or
        /// Maximize, whichever a double-click would do).
        bold: bool,
        /// Caption-button glyph in the gutter, drawn in the text color so it
        /// inverts with the highlight; used instead of an icon.
        glyph: Option<MenuGlyph>,
    },
    Separator,
}

pub use crate::art::glyph::Glyph as MenuGlyph;

impl MenuItem {
    pub fn entry(label: impl Into<String>) -> MenuItem {
        MenuItem::Entry {
            label: label.into(),
            icon: None,
            enabled: true,
            checked: false,
            radio: false,
            submenu: false,
            mnemonic: None,
            shortcut: None,
            bold: false,
            glyph: None,
        }
    }

    pub fn item(label: &str) -> MenuItem {
        let (label, mnemonic) = split_mnemonic(label);
        MenuItem::entry(label).mnemonic(mnemonic)
    }

    pub fn icon(mut self, icon: Option<Canvas>) -> MenuItem {
        if let MenuItem::Entry { icon: i, .. } = &mut self {
            *i = icon;
        }
        self
    }
    pub fn enabled(mut self, on: bool) -> MenuItem {
        if let MenuItem::Entry { enabled, .. } = &mut self {
            *enabled = on;
        }
        self
    }
    pub fn checked(mut self, on: bool) -> MenuItem {
        if let MenuItem::Entry { checked, .. } = &mut self {
            *checked = on;
        }
        self
    }
    pub fn radio(mut self, on: bool) -> MenuItem {
        if let MenuItem::Entry { radio, .. } = &mut self {
            *radio = on;
        }
        self
    }
    pub fn submenu(mut self, on: bool) -> MenuItem {
        if let MenuItem::Entry { submenu, .. } = &mut self {
            *submenu = on;
        }
        self
    }
    pub fn mnemonic(mut self, at: Option<usize>) -> MenuItem {
        if let MenuItem::Entry { mnemonic, .. } = &mut self {
            *mnemonic = at;
        }
        self
    }
    pub fn shortcut(mut self, text: Option<String>) -> MenuItem {
        if let MenuItem::Entry { shortcut, .. } = &mut self {
            *shortcut = text;
        }
        self
    }
    pub fn bold(mut self, on: bool) -> MenuItem {
        if let MenuItem::Entry { bold, .. } = &mut self {
            *bold = on;
        }
        self
    }
    pub fn glyph(mut self, g: Option<MenuGlyph>) -> MenuItem {
        if let MenuItem::Entry { glyph, .. } = &mut self {
            *glyph = g;
        }
        self
    }

    pub fn access_key(&self) -> Option<char> {
        let MenuItem::Entry {
            label,
            enabled: true,
            mnemonic,
            ..
        } = self
        else {
            return None;
        };
        let ch = match mnemonic {
            Some(i) => label.chars().nth(*i),
            None => label.chars().find(|c| c.is_alphanumeric()),
        };
        ch.and_then(|c| c.to_lowercase().next())
    }

    fn selectable(&self) -> bool {
        matches!(self, MenuItem::Entry { enabled: true, .. })
    }
}

/// `label` with `{user}` replaced by `user`, so a config row can read
/// "Log Off {user}...".
pub fn expand_label(label: &str, user: &str) -> String {
    label.replace("{user}", user)
}

/// Which rows the arrow keys stop on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyStops {
    /// Enabled entries only: greyed rows are passed over.
    Enabled,
    /// Every entry, greyed ones too, as the pointer lights them; Enter on a greyed row does nothing.
    Entries,
}

impl KeyStops {
    /// A look with its own menus (Glass) stops on greyed rows; Classic and Blue pass over them.
    pub fn of(theme: &Theme) -> KeyStops {
        if theme.look.style().is_some_and(|st| st.menu.is_some()) {
            KeyStops::Entries
        } else {
            KeyStops::Enabled
        }
    }
}

pub fn step_selection(
    items: &[MenuItem],
    current: Option<usize>,
    step: i32,
    stops: KeyStops,
) -> Option<usize> {
    let n = items.len() as i32;
    if n == 0 {
        return None;
    }
    let start = match current {
        Some(i) => i as i32,
        None if step > 0 => -1,
        None => n,
    };
    (1..=n)
        .map(|k| (start + k * step).rem_euclid(n) as usize)
        .find(|&i| match stops {
            KeyStops::Enabled => items[i].selectable(),
            KeyStops::Entries => matches!(items[i], MenuItem::Entry { .. }),
        })
}

pub fn default_row(items: &[MenuItem]) -> Option<usize> {
    items.iter().position(|it| {
        matches!(
            it,
            MenuItem::Entry {
                bold: true,
                enabled: true,
                ..
            }
        )
    })
}

pub fn enter_row(items: &[MenuItem], current: Option<usize>) -> Option<usize> {
    current.or_else(|| default_row(items))
}

pub fn rows_for_key(items: &[MenuItem], key: char) -> impl Iterator<Item = usize> + '_ {
    let key = key.to_lowercase().next().unwrap_or(key);
    items
        .iter()
        .enumerate()
        .filter(move |(_, it)| it.access_key() == Some(key))
        .map(|(i, _)| i)
}

/// Inset of a menu's rows from its edges: the border plus its pad. The
/// Start menu's comes from its look ([`StartMenuMetrics`]).
const BORDER: i32 = 3;
const INSET: Insets = Insets::new(BORDER, BORDER, BORDER, BORDER);
const SEPARATOR_H: i32 = 8;
const ICON_X: i32 = 0;
const TEXT_X: i32 = 24;
const ROOT_ICON_X: i32 = 6;
const ROOT_TEXT_X: i32 = 42;
pub const CHECK_X: i32 = 4;
/// How far below the centred position the check glyph sits: its top
/// row is 6 px down a 17 px row.
const CHECK_DY: i32 = 1;
pub const BULLET_X: i32 = CHECK_X + 1;
pub const ARROW_RIGHT_INSET: i32 = 9;
const ARROW_SPACE: i32 = ARROW_RIGHT_INSET + 4;
/// Space between the longest label's end and the menu's right edge on a
/// Start submenu (arrow column and margins), less the border `fit` adds. A
/// context menu's is its style's `right_pad`.
const RIGHT_PAD: i32 = 20;
const ROOT_RIGHT_PAD: i32 = 26;
const BANNER_PAD: i32 = 4;
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

    fn compact(m: &Metrics) -> MenuStyle {
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

const CHECK_GLYPH: [&str; 7] = [
    "......X", ".....XX", "X...XXX", "XX.XXX.", "XXXXX..", ".XXX...", "..X....",
];

fn draw_check(canvas: &mut Canvas, x: i32, y: i32, s: i32, color: crate::theme::Color) {
    canvas.paint_map(x, y, s, &CHECK_GLYPH, |ch| (ch == 'X').then_some(color));
}

pub const BULLET_D: i32 = 6;
const BULLET_RUNS: [(i32, i32); BULLET_D as usize] =
    [(1, 4), (0, 6), (0, 6), (0, 6), (0, 6), (1, 4)];

fn draw_bullet(canvas: &mut Canvas, x: i32, y: i32, s: i32, color: crate::theme::Color) {
    for (row, (off, len)) in BULLET_RUNS.iter().enumerate() {
        canvas.fill_rect(
            Rect::new(x + off * s, y + row as i32 * s, len * s, s),
            color,
        );
    }
}

fn draw_arrow(canvas: &mut Canvas, x: i32, y: i32, s: i32, color: crate::theme::Color) {
    for col in 0..4 {
        let h = 7 - 2 * col;
        canvas.fill_rect(Rect::new(x + col * s, y + col * s, s, h * s), color);
    }
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

/// A menu whose rows can open menus of their own: `children[i]` is the submenu row `i` opens.
/// Build it through [`MenuTree::push`] and [`MenuTree::push_sub`] so the two vectors stay the same length.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MenuTree {
    items: Vec<MenuItem>,
    children: Vec<Option<MenuTree>>,
}

impl MenuTree {
    pub fn flat(items: Vec<MenuItem>) -> MenuTree {
        let children = items.iter().map(|_| None).collect();
        MenuTree { items, children }
    }

    pub fn new() -> MenuTree {
        MenuTree::default()
    }

    pub fn push(&mut self, item: MenuItem) {
        self.items.push(item);
        self.children.push(None);
    }

    /// A row that opens `child`; the row gets the arrow whether or not
    /// the caller asked for it.
    pub fn push_sub(&mut self, mut item: MenuItem, child: MenuTree) {
        if let MenuItem::Entry { submenu, .. } = &mut item {
            *submenu = true;
        }
        self.items.push(item);
        self.children.push(Some(child));
    }

    pub fn items(&self) -> &[MenuItem] {
        &self.items
    }

    pub fn child(&self, row: usize) -> Option<&MenuTree> {
        self.children.get(row)?.as_ref()
    }

    pub fn at(&self, path: &[usize]) -> Option<&MenuTree> {
        match path.split_first() {
            None => Some(self),
            Some((&row, rest)) => self.child(row)?.at(rest),
        }
    }
}

impl From<Vec<MenuItem>> for MenuTree {
    fn from(items: Vec<MenuItem>) -> MenuTree {
        MenuTree::flat(items)
    }
}

/// Reading and editing the rows is what a flat menu did; the slice cannot
/// resize, so the children stay paired with them.
impl std::ops::Deref for MenuTree {
    type Target = [MenuItem];

    fn deref(&self) -> &[MenuItem] {
        &self.items
    }
}

impl std::ops::DerefMut for MenuTree {
    fn deref_mut(&mut self) -> &mut [MenuItem] {
        &mut self.items
    }
}

#[derive(Clone, Copy)]
pub struct MenuCtx<'a> {
    pub text: &'a TextRenderer,
    pub theme: &'a Theme,
    pub px: u32,
    pub within: Rect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKey {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Escape,
    Char(char),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuAction {
    /// Not the menu's: the host decides (Left on the root menu of a menu
    /// bar walks to the previous title, say).
    None,
    Redraw,
    Chosen(Vec<usize>),
    Close,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Level {
    pub path: Vec<usize>,
    pub items: Vec<MenuItem>,
    pub origin: (i32, i32),
    pub layout: MenuLayout,
    pub hovered: Option<usize>,
}

impl Level {
    pub fn rect(&self) -> Rect {
        Rect::new(
            self.origin.0,
            self.origin.1,
            self.layout.width,
            self.layout.height,
        )
    }

    /// `Some(row)` over an entry, `Some(None)` over a separator or the
    /// border, `None` off the menu.
    fn hit(&self, x: i32, y: i32) -> Option<Option<usize>> {
        if !self.rect().contains(x, y) {
            return None;
        }
        Some(self.layout.hit(x - self.origin.0, y - self.origin.1))
    }

    fn opens_submenu(&self, row: usize) -> bool {
        matches!(
            self.items.get(row),
            Some(MenuItem::Entry {
                submenu: true,
                enabled: true,
                ..
            })
        )
    }
}

///
/// The host owns the submenu hover delay: [`PopupMenu::motion`] opens a
/// hovered row's submenu at once, so a host that wants the delay calls it
/// from its own timer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopupMenu {
    levels: Vec<Level>,
    /// Opened from the keyboard: the chain underlines its access keys
    /// whatever its host's cues say.
    pub(crate) keys: bool,
}

fn level_layout(items: &[MenuItem], cx: &MenuCtx) -> MenuLayout {
    let style = MenuStyle::context_for(cx.theme, items).fit(cx.text, items, cx.px, cx.within.width);
    MenuLayout::compute_styled(&style, items, usize::MAX)
}

impl PopupMenu {
    pub fn open(tree: &MenuTree, at: (i32, i32), cx: &MenuCtx, by_keyboard: bool) -> PopupMenu {
        let items = tree.items.clone();
        let layout = level_layout(&items, cx);
        let x = at.0.min(cx.within.right() - layout.width).max(cx.within.x);
        let y =
            at.1.min(cx.within.bottom() - layout.height)
                .max(cx.within.y);
        let hovered = by_keyboard
            .then(|| step_selection(&items, None, 1, KeyStops::of(cx.theme)))
            .flatten();
        PopupMenu {
            levels: vec![Level {
                path: Vec::new(),
                items,
                origin: (x, y),
                layout,
                hovered,
            }],
            keys: by_keyboard,
        }
    }

    pub fn levels(&self) -> &[Level] {
        &self.levels
    }

    pub fn depth(&self) -> usize {
        self.levels.len()
    }

    pub fn rect(&self) -> Rect {
        self.levels[0].rect()
    }

    pub fn hovered(&self) -> Option<usize> {
        self.levels.last().and_then(|l| l.hovered)
    }

    /// The level and row under a point, deepest menu first (a child
    /// overlaps its parent). `Some((level, None))` is the border or a
    /// separator; `None` is off the chain entirely.
    pub fn hit(&self, x: i32, y: i32) -> Option<(usize, Option<usize>)> {
        self.levels
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, l)| l.hit(x, y).map(|row| (i, row)))
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.hit(x, y).is_some()
    }

    fn open_child(
        &mut self,
        tree: &MenuTree,
        level: usize,
        row: usize,
        cx: &MenuCtx,
        by_keyboard: bool,
    ) -> bool {
        let Some(parent) = self.levels.get(level) else {
            return false;
        };
        let mut path = parent.path.clone();
        path.push(row);
        let (Some(child), Some(r)) = (tree.at(&path), parent.layout.rows.get(row).copied()) else {
            return false;
        };
        let items = child.items.clone();
        let layout = level_layout(&items, cx);
        let prect = parent.rect();
        let row_abs = Rect::new(prect.x + r.x, prect.y + r.y, r.width, r.height);
        let (x, y) = place_submenu(prect, row_abs, (layout.width, layout.height), cx.within);
        let hovered = by_keyboard
            .then(|| step_selection(&items, None, 1, KeyStops::of(cx.theme)))
            .flatten();
        self.levels.truncate(level + 1);
        self.levels.push(Level {
            path,
            items,
            origin: (x, y),
            layout,
            hovered,
        });
        true
    }

    fn child_open_for(&self, level: usize, row: Option<usize>) -> bool {
        match (row, self.levels.get(level + 1)) {
            (Some(r), Some(child)) => child.path.last() == Some(&r),
            _ => false,
        }
    }

    /// Hovering a submenu row opens it (see the type's doc on the delay),
    /// and leaving it for another row of the same menu closes it again.
    pub fn motion(&mut self, tree: &MenuTree, x: i32, y: i32, cx: &MenuCtx) -> bool {
        let Some((level, row)) = self.hit(x, y) else {
            return false;
        };
        let mut dirty = false;
        if self.levels[level].hovered != row {
            self.levels[level].hovered = row;
            dirty = true;
        }
        if self.child_open_for(level, row) {
            return dirty;
        }
        if self.levels.len() > level + 1 {
            self.levels.truncate(level + 1);
            dirty = true;
        }
        if let Some(r) = row
            && self.levels[level].opens_submenu(r)
        {
            dirty |= self.open_child(tree, level, r, cx, false);
        }
        dirty
    }

    pub fn key(&mut self, tree: &MenuTree, key: MenuKey, cx: &MenuCtx) -> MenuAction {
        let last = self.levels.len() - 1;
        match key {
            MenuKey::Up | MenuKey::Down => {
                let step = if key == MenuKey::Down { 1 } else { -1 };
                let level = &mut self.levels[last];
                level.hovered =
                    step_selection(&level.items, level.hovered, step, KeyStops::of(cx.theme));
                MenuAction::Redraw
            }
            MenuKey::Right => match self.levels[last].hovered {
                Some(r) if self.levels[last].opens_submenu(r) => {
                    self.open_child(tree, last, r, cx, true);
                    MenuAction::Redraw
                }
                // Not a submenu row: the host's (the next menu bar title).
                _ => MenuAction::None,
            },
            MenuKey::Left => {
                if last > 0 {
                    self.levels.truncate(last);
                    MenuAction::Redraw
                } else {
                    MenuAction::None
                }
            }
            MenuKey::Escape => {
                if last > 0 {
                    self.levels.truncate(last);
                    MenuAction::Redraw
                } else {
                    MenuAction::Close
                }
            }
            // With nothing highlighted, Enter takes the bold default row.
            MenuKey::Enter => {
                let row = enter_row(&self.levels[last].items, self.levels[last].hovered);
                self.levels[last].hovered = row;
                self.pick(tree, last, row, cx)
            }
            MenuKey::Char(c) => {
                let row = rows_for_key(&self.levels[last].items, c).next();
                match row {
                    Some(r) => {
                        self.levels[last].hovered = Some(r);
                        self.pick(tree, last, Some(r), cx)
                    }
                    None => MenuAction::None,
                }
            }
        }
    }

    pub fn click(&mut self, tree: &MenuTree, x: i32, y: i32, cx: &MenuCtx) -> MenuAction {
        match self.hit(x, y) {
            None => MenuAction::Close,
            Some((level, None)) => {
                self.levels.truncate(level + 1);
                MenuAction::Redraw
            }
            Some((level, Some(row))) => {
                self.levels[level].hovered = Some(row);
                self.pick(tree, level, Some(row), cx)
            }
        }
    }

    fn pick(
        &mut self,
        tree: &MenuTree,
        level: usize,
        row: Option<usize>,
        cx: &MenuCtx,
    ) -> MenuAction {
        let Some(row) = row else {
            return MenuAction::Redraw;
        };
        if !self.levels[level]
            .items
            .get(row)
            .is_some_and(MenuItem::selectable)
        {
            return MenuAction::None;
        }
        if self.levels[level].opens_submenu(row) {
            if !self.child_open_for(level, Some(row)) {
                self.open_child(tree, level, row, cx, true);
            }
            return MenuAction::Redraw;
        }
        let mut path = self.levels[level].path.clone();
        path.push(row);
        MenuAction::Chosen(path)
    }

    /// Under a look whose menus cast a shadow
    /// ([`MenuLook::shadow`](crate::style::MenuLook::shadow)) each level
    /// casts it over what lies under it first.
    /// `underline` is the host's cue; a chain opened from the keyboard underlines regardless.
    pub fn draw(&self, c: &mut Canvas, theme: &Theme, text: &TextRenderer, underline: bool) {
        for i in 0..self.levels.len() {
            self.draw_level(i, (0, 0), c, theme, text, underline);
        }
    }

    /// Level `i` alone with its shadow, moved `by` logical px: a host that shows each level on
    /// its own surface draws it there at its corner.
    pub fn draw_level(
        &self,
        i: usize,
        by: (i32, i32),
        c: &mut Canvas,
        theme: &Theme,
        text: &TextRenderer,
        underline: bool,
    ) {
        let Some(level) = self.levels.get(i) else {
            return;
        };
        let underline = underline || self.keys;
        let s = theme.scale.max(1);
        let (x, y) = (level.origin.0 + by.0, level.origin.1 + by.1);
        if let Some(m) = menu_look(theme) {
            let r = Rect::new(x, y, level.layout.width, level.layout.height).scaled(s);
            crate::look::styled::menu::draw_shadow(&m, c, r, s);
        }
        draw_menu_at(
            c,
            (x * s, y * s),
            theme,
            text,
            &level.layout.scaled(s),
            &level.items,
            level.hovered,
            "",
            underline,
        );
    }

    /// Moves level `i` to `origin`, where its host's compositor put it.
    pub fn place(&mut self, i: usize, origin: (i32, i32)) {
        if let Some(level) = self.levels.get_mut(i) {
            level.origin = origin;
        }
    }

    /// Where level `i` (a submenu) opens against its parent, for a host that lets the
    /// compositor place it: an anchor rect in the parent level's own coordinates and the
    /// child's offset down from it. The child's left edge sits on the rect's right end, or
    /// flipped, its right edge on the rect's left end, as [`place_submenu`] puts it.
    pub fn submenu_anchor(&self, i: usize) -> Option<(Rect, i32)> {
        let child = self.levels.get(i).filter(|_| i > 0)?;
        let parent = &self.levels[i - 1];
        let row = parent.layout.rows.get(*child.path.last()?)?;
        let overlap = BORDER + INSET.left;
        let w = (parent.layout.width - 2 * overlap).max(1);
        Some((Rect::new(overlap, row.y, w, row.height), -INSET.top))
    }
}

fn band_stops(cs: &crate::theme::ColorScheme) -> [(f32, crate::theme::Color); 7] {
    [
        (0.00, cs.menu_band),
        (0.04, cs.menu_band),
        (0.22, cs.gradient_menu_band),
        (0.45, cs.gradient_menu_band),
        (0.75, cs.menu_band),
        (0.82, cs.menu_band_top),
        (1.00, cs.menu_band_top),
    ]
}

fn band_color(stops: &[(f32, crate::theme::Color)], t: f32) -> crate::theme::Color {
    let mut prev = stops[0];
    for &next in &stops[1..] {
        if t <= next.0 {
            let span = (next.0 - prev.0).max(f32::EPSILON);
            let k = ((t - prev.0) / span).clamp(0.0, 1.0);
            let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * k).round() as u8;
            return crate::theme::Color::rgb(
                mix(prev.1.r, next.1.r),
                mix(prev.1.g, next.1.g),
                mix(prev.1.b, next.1.b),
            );
        }
        prev = next;
    }
    prev.1
}

fn draw_band(canvas: &mut Canvas, theme: &Theme, text: &TextRenderer, band: Rect, banner: &str) {
    let cs = &theme.colors;
    let s = theme.scale.max(1);
    // Drawn sideways (text left to right, gradient bottom colour at the
    // left) and then turned so the left edge becomes the bottom.
    let mut strip = Canvas::new(band.height as u32, band.width as u32);
    let stops = band_stops(cs);
    let denom = (band.height - 1).max(1) as f32;
    for x in 0..band.height {
        strip.vline(x, 0, band.width, band_color(&stops, x as f32 / denom));
    }
    let size = theme.fonts.size_px + 3 * (s as u32);
    let (bold, regular) = match banner.rsplit_once(' ') {
        Some((head, tail)) => (head.to_string(), format!(" {tail}")),
        None => (banner.to_string(), String::new()),
    };
    let mut pen = BANNER_PAD * s;
    for (part, is_bold) in [(bold, true), (regular, false)] {
        if part.is_empty() {
            continue;
        }
        text.draw(
            &mut strip,
            Rect::new(pen, 0, (band.height - pen).max(0), band.width),
            &part,
            is_bold,
            cs.menu_band_text,
            Align::Left,
            size,
        );
        pen += text.measure(&part, is_bold, size);
    }
    canvas.blit_canvas(band.x, band.y, &strip.rotate_ccw());
}

const SHADOW_ALPHA: f32 = 0.5;

pub fn draw_shadow(w: i32, h: i32, size: i32) -> Canvas {
    let mut c = Canvas::new((w + size).max(1) as u32, (h + size).max(1) as u32);
    if size <= 0 {
        return c;
    }
    let fade = |past: i32| -> f32 { (size - past) as f32 / size as f32 };
    let mut put = |x: i32, y: i32| {
        let fx = if x >= w { fade(x - w) } else { 1.0 };
        let fy = if y >= h { fade(y - h) } else { 1.0 };
        let a = (SHADOW_ALPHA * fx * fy * 255.0).round() as u32;
        c.put_raw(x, y, a << 24);
    };
    for y in size..h + size {
        for x in w.max(size)..w + size {
            put(x, y);
        }
    }
    for y in h.max(size)..h + size {
        for x in size..w {
            put(x, y);
        }
    }
    c
}

pub fn draw_shadow_above(w: i32, h: i32, size: i32, floor: i32) -> Canvas {
    cut_at(draw_shadow(w, h, size), floor)
}

fn cut_at(full: Canvas, floor: i32) -> Canvas {
    let rows = floor.clamp(1, full.height as i32);
    let mut c = Canvas::new(full.width, rows as u32);
    c.blit_over(0, 0, &full);
    c
}

fn menu_look(theme: &Theme) -> Option<crate::style::MenuLook> {
    theme.look.style().and_then(|st| st.menu)
}

/// Whether [`PopupMenu::draw`] casts `theme`'s menu shadow itself (or none, for a menu look
/// without one); a host that draws Classic's ([`draw_shadow`]) under a chain skips it then.
pub fn draws_own_shadow(theme: &Theme) -> bool {
    menu_look(theme).is_some()
}

/// How far right and down of a [`PopupMenu`] level its own shadow reaches, logical px.
pub fn level_shadow(theme: &Theme) -> i32 {
    match menu_look(theme).and_then(|m| m.shadow) {
        Some(_) => crate::style::DropShadow::N as i32,
        None => 0,
    }
}

pub fn shadow_reach(theme: &Theme) -> i32 {
    match menu_look(theme) {
        Some(m) => m.shadow.map_or(0, |_| {
            crate::style::DropShadow::N as i32 * theme.scale.max(1)
        }),
        None => theme.metrics.menu_shadow,
    }
}

pub fn draw_menu_shadow(theme: &Theme, w: i32, h: i32, floor: i32) -> Canvas {
    let reach = shadow_reach(theme);
    match menu_look(theme) {
        Some(m) => {
            let s = theme.scale.max(1);
            let mut c = Canvas::new((w + reach).max(1) as u32, (h + reach).max(1) as u32);
            crate::look::styled::menu::draw_shadow(&m, &mut c, Rect::new(0, 0, w, h), s);
            cut_at(c, floor)
        }
        None => draw_shadow_above(w, h, reach, floor),
    }
}

/// The top of an `h` px (logical) glyph centred down row `r` at scale
/// `s`, rounded in logical pixels so a scale-2 row is its scale-1 row
/// doubled.
fn centred(r: &Rect, h: i32, s: i32) -> i32 {
    r.y + (r.height / s - h) / 2 * s
}

#[allow(clippy::too_many_arguments)]
pub fn draw_menu(
    canvas: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    layout: &MenuLayout,
    items: &[MenuItem],
    hovered: Option<usize>,
    banner: &str,
    underline: bool,
) {
    draw_menu_at(
        canvas,
        (0, 0),
        theme,
        text,
        layout,
        items,
        hovered,
        banner,
        underline,
    );
}

#[allow(clippy::too_many_arguments)]
pub fn draw_menu_at(
    canvas: &mut Canvas,
    at: (i32, i32),
    theme: &Theme,
    text: &TextRenderer,
    layout: &MenuLayout,
    items: &[MenuItem],
    hovered: Option<usize>,
    banner: &str,
    underline: bool,
) {
    debug_assert_eq!(
        items.len(),
        layout.rows.len(),
        "menu layout does not match items"
    );
    if layout.gutter > 0
        && let Some(st) = theme.look.style()
        && let Some(m) = &st.menu
    {
        return draw_gutter_menu(
            canvas, at, theme, st, m, text, layout, items, hovered, underline,
        );
    }
    let cs = &theme.colors;
    let s = theme.scale.max(1);
    let size = theme.fonts.size_px;
    let (ox, oy) = at;
    let flat = theme.look.style().is_some_and(|st| st.flat_menus);
    let all = Rect::new(ox, oy, layout.width, layout.height);
    canvas.fill_rect(all, cs.menu);
    if flat {
        canvas.ring_t(all, cs.button_shadow, cs.button_shadow, s);
    } else {
        canvas.edge_t(all, EdgeStyle::WindowRaised, cs, s);
    }
    if layout.band > 0 {
        // A flat menu's band reaches out to its 1 px border; Classic's
        // starts where the rows' inset does.
        let (x0, y0, y1) = if flat {
            (s, s, s)
        } else {
            (layout.inset.left, layout.inset.top, layout.inset.bottom)
        };
        let band = Rect::new(
            ox + x0,
            oy + y0,
            layout.band + layout.inset.left - x0,
            layout.height - y0 - y1,
        );
        draw_band(canvas, theme, text, band, banner);
    }
    for c in 1..layout.columns {
        let x = ox + layout.band + c as i32 * layout.column_width - s;
        if flat {
            let line = Rect::new(x, oy + s, s, layout.height - 2 * s);
            canvas.fill_rect(line, cs.button_shadow);
        } else {
            canvas.etched_vline_t(x, oy + s, oy + layout.height - s, cs, s);
        }
    }
    for (i, (item, r)) in items.iter().zip(&layout.rows).enumerate() {
        let r = &Rect::new(r.x + ox, r.y + oy, r.width, r.height);
        match item {
            MenuItem::Separator => {
                let y = r.y + (r.height / s / 2 - 1) * s;
                let dx = layout.separator_x;
                if flat {
                    let line = Rect::new(r.x + dx, y, r.width - 2 * dx, s);
                    canvas.fill_rect(line, cs.button_shadow);
                } else {
                    canvas.etched_hline_t(r.x + dx, r.right() - dx, y, cs, s);
                }
            }
            MenuItem::Entry {
                label,
                icon,
                enabled,
                checked,
                radio,
                submenu,
                mnemonic,
                shortcut,
                bold,
                glyph,
            } => {
                // A disabled row highlights too, its text staying grey.
                // A flat menu's lit row is `menu_hilight` in a 1 px
                // `hilight` frame.
                let hovered_here = hovered == Some(i);
                let lit = if flat { cs.menu_hilight } else { cs.hilight };
                let (bg, fg) = if hovered_here && *enabled {
                    (lit, cs.hilight_text)
                } else if hovered_here {
                    (lit, cs.gray_text)
                } else if *enabled {
                    (cs.menu, cs.menu_text)
                } else {
                    (cs.menu, cs.gray_text)
                };
                canvas.fill_rect(*r, bg);
                if flat && hovered_here {
                    canvas.ring_t(*r, cs.hilight, cs.hilight, s);
                }
                // Classic greys a disabled label by etching it; a flat
                // menu draws it grey alone.
                let etched = !*enabled && !hovered_here && !flat;
                let glyph_y = centred(r, 7, s);
                match icon {
                    Some(ic) => {
                        let iy = r.y + (r.height - ic.height as i32) / 2;
                        canvas.blit_canvas(r.x + layout.icon_x, iy, ic);
                    }
                    None if *checked && *radio => {
                        draw_bullet(canvas, r.x + BULLET_X * s, centred(r, BULLET_D, s), s, fg)
                    }
                    None if *checked => {
                        draw_check(canvas, r.x + CHECK_X * s, glyph_y + CHECK_DY * s, s, fg)
                    }
                    None => {
                        if let Some(g) = glyph {
                            let gy = centred(r, 14, s);
                            crate::art::glyph::draw_glyph_strokes(
                                canvas,
                                r.x + layout.icon_x,
                                gy,
                                s,
                                *g,
                                fg,
                                bg,
                            );
                        }
                    }
                }
                if *submenu {
                    let ink = if hovered_here && *enabled && layout.lit_arrow_menu_text {
                        cs.menu_text
                    } else {
                        fg
                    };
                    draw_arrow(canvas, r.right() - ARROW_RIGHT_INSET * s, glyph_y, s, ink);
                }
                let text_rect = Rect::new(
                    r.x + layout.text_x,
                    r.y,
                    (r.width - layout.text_x - ARROW_SPACE * s).max(0),
                    r.height,
                );
                let shown = text.ellipsize(label, text_rect.width, *bold, size);
                if let Some(sc) = shortcut {
                    let sc_rect = Rect::new(text_rect.x, r.y, text_rect.width, r.height);
                    if etched {
                        let lit =
                            Rect::new(sc_rect.x + s, sc_rect.y + s, sc_rect.width, sc_rect.height);
                        text.draw(
                            canvas,
                            lit,
                            sc,
                            false,
                            cs.button_hilight,
                            Align::Right,
                            size,
                        );
                    }
                    text.draw(canvas, sc_rect, sc, false, fg, Align::Right, size);
                }
                if etched {
                    let lit = Rect::new(
                        text_rect.x + s,
                        text_rect.y + s,
                        text_rect.width,
                        text_rect.height,
                    );
                    text.draw(
                        canvas,
                        lit,
                        &shown,
                        *bold,
                        cs.button_hilight,
                        Align::Left,
                        size,
                    );
                }
                let mnemonic = mnemonic.filter(|_| underline);
                draw_label(
                    canvas,
                    text,
                    text_rect,
                    &shown,
                    matches!(shown, Cow::Owned(_)),
                    *bold,
                    mnemonic,
                    fg,
                    size,
                    s,
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_label(
    canvas: &mut Canvas,
    text: &TextRenderer,
    r: Rect,
    shown: &str,
    ellipsized: bool,
    bold: bool,
    mnemonic: Option<usize>,
    fg: crate::theme::Color,
    size: u32,
    s: i32,
) {
    text.draw(canvas, r, shown, bold, fg, Align::Left, size);
    // An ellipsized label keeps a prefix of `label` before its three
    // dots; an access key past that prefix is not drawn.
    let visible = if ellipsized {
        shown.chars().count() - 3
    } else {
        usize::MAX
    };
    if let Some(m) = mnemonic.filter(|&m| m < visible) {
        // The access-key underline sits one pixel under the baseline, `s` thick.
        if let Some(baseline) = text.baseline(r, bold, size) {
            let (dx, w) = text.span(shown, m, bold, size);
            let ul = Rect::new(r.x + dx, baseline + s, w, s);
            canvas.fill_rect(ul.intersect(&r), fg);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_gutter_menu(
    canvas: &mut Canvas,
    (ox, oy): (i32, i32),
    theme: &Theme,
    st: &crate::style::Style,
    m: &crate::style::MenuLook,
    text: &TextRenderer,
    layout: &MenuLayout,
    items: &[MenuItem],
    hovered: Option<usize>,
    underline: bool,
) {
    use crate::look::styled::menu as look;
    let cs = &theme.colors;
    let s = theme.scale.max(1);
    let size = theme.fonts.size_px;
    let all = Rect::new(ox, oy, layout.width, layout.height);
    look::draw_frame(m, canvas, all, cs.menu, s);
    let ins = layout.inset;
    let rows_h = layout.height - ins.top - ins.bottom;
    for c in 0..layout.columns as i32 {
        let x = ox + layout.band + c * layout.column_width + ins.left;
        canvas.fill_rect(
            Rect::new(x, oy + ins.top, layout.gutter, rows_h),
            m.gutter_fill,
        );
        let dx = x + layout.gutter;
        canvas.fill_rect(Rect::new(dx, oy + ins.top, s, rows_h), m.divider.0);
        canvas.fill_rect(Rect::new(dx + s, oy + ins.top, s, rows_h), m.divider.1);
        if c > 0 {
            let x = x - ins.left;
            let line = |x: i32| Rect::new(x, oy + ins.top, s, rows_h);
            canvas.fill_rect(line(x - s), m.rule.0);
            canvas.fill_rect(line(x), m.rule.1);
        }
    }
    let past_divider = layout.gutter + 2 * s;
    for (i, (item, r)) in items.iter().zip(&layout.rows).enumerate() {
        let r = Rect::new(r.x + ox, r.y + oy, r.width, r.height);
        let MenuItem::Entry {
            label,
            icon,
            enabled,
            checked,
            radio,
            submenu,
            mnemonic,
            shortcut,
            bold,
            glyph,
        } = item
        else {
            let line =
                |dy: i32| Rect::new(r.x + past_divider, r.y + dy * s, r.width - past_divider, s);
            let mid = m.separator / 2;
            canvas.fill_rect(line(mid), m.rule.0);
            canvas.fill_rect(line(mid + 1), m.rule.1);
            continue;
        };
        let lit = hovered == Some(i);
        if lit {
            look::draw_lit_row(st, canvas, r, !*enabled, s);
        }
        let fg = if *enabled { cs.menu_text } else { m.greyed };
        let square = Rect::new(r.x, r.y, r.height, r.height);
        match icon {
            Some(ic) => {
                let iy = r.y + (r.height - ic.height as i32) / 2;
                canvas.blit_canvas(r.x + layout.icon_x, iy, ic);
            }
            None if *checked => {
                look::draw_check(st, m, canvas, square, *radio, lit, s);
            }
            None => {
                if let Some(g) = glyph {
                    let gy = centred(&r, 14, s);
                    let bg = canvas.get(r.x + layout.icon_x, gy);
                    crate::art::glyph::draw_glyph_strokes(
                        canvas,
                        r.x + layout.icon_x,
                        gy,
                        s,
                        *g,
                        fg,
                        crate::theme::Color::rgb((bg >> 16) as u8, (bg >> 8) as u8, bg as u8),
                    );
                }
            }
        }
        if *submenu {
            let ink = if *enabled { m.arrow } else { m.greyed };
            draw_arrow(
                canvas,
                r.right() - m.arrow_inset * s,
                centred(&r, 7, s),
                s,
                ink,
            );
        }
        let text_rect = Rect::new(
            r.x + layout.text_x,
            r.y,
            (r.width - layout.text_x - m.shortcut_end * s).max(0),
            r.height,
        );
        let shown = text.ellipsize(label, text_rect.width, *bold, size);
        if let Some(sc) = shortcut {
            text.draw(canvas, text_rect, sc, false, fg, Align::Right, size);
        }
        let mnemonic = mnemonic.filter(|_| underline);
        draw_label(
            canvas,
            text,
            text_rect,
            &shown,
            matches!(shown, Cow::Owned(_)),
            *bold,
            mnemonic,
            fg,
            size,
            s,
        );
    }
}

pub fn access_row(item: &MenuItem, r: Rect, open: bool) -> Option<Node> {
    let MenuItem::Entry {
        label,
        enabled,
        checked,
        radio,
        submenu,
        shortcut,
        ..
    } = item
    else {
        return None;
    };
    let role = match (radio, checked) {
        (true, _) => Role::MenuItemRadio,
        (false, true) => Role::MenuItemCheckBox,
        (false, false) => Role::MenuItem,
    };
    let mut n = access::node(role, r);
    n.set_label(label.as_str());
    if role != Role::MenuItem {
        n.set_toggled((*checked).into());
    }
    if *submenu {
        n.set_has_popup(accesskit::HasPopup::Menu);
        n.set_expanded(open);
    }
    if let Some(s) = shortcut {
        n.set_keyboard_shortcut(s.as_str());
    }
    access::focusable(&mut n, *enabled);
    Some(n)
}

/// Returns the menu's id, the id each row got (the menu's own for a separator, so a submenu hung there has a parent), and the highlighted row's id.
#[allow(clippy::too_many_arguments)]
pub fn access_menu(
    tree: &mut Tree,
    parent: NodeId,
    key: impl std::hash::Hash,
    items: &[MenuItem],
    layout: &MenuLayout,
    rect: Rect,
    hovered: Option<usize>,
    open: Option<usize>,
) -> (NodeId, Vec<NodeId>, Option<NodeId>) {
    let menu = tree.push(parent, key, access::node(Role::Menu, rect));
    let mut rows = Vec::with_capacity(items.len());
    let mut focus = None;
    for (i, item) in items.iter().enumerate() {
        let r = layout.rows.get(i).copied().unwrap_or_default();
        let r = Rect::new(r.x + rect.x, r.y + rect.y, r.width, r.height);
        let Some(n) = access_row(item, r, open == Some(i)) else {
            rows.push(menu);
            continue;
        };
        let id = tree.push(menu, i, n);
        rows.push(id);
        if hovered == Some(i) {
            focus = Some(id);
        }
    }
    (menu, rows, focus)
}

impl PopupMenu {
    pub fn access(&self, tree: &mut Tree, parent: NodeId, key: impl std::hash::Hash) -> NodeId {
        let mut up = parent;
        let mut focus = parent;
        let mut rows: Vec<NodeId> = Vec::new();
        for (k, level) in self.levels.iter().enumerate() {
            if k > 0
                && let Some(&row) = level.path.last().and_then(|&i| rows.get(i))
            {
                up = row;
            }
            let next = self.levels.get(k + 1).and_then(|l| l.path.last().copied());
            let (menu, ids, hot) = access_menu(
                tree,
                up,
                (&key, k),
                &level.items,
                &level.layout,
                level.rect(),
                level.hovered,
                next,
            );
            focus = hot.unwrap_or(menu);
            rows = ids;
        }
        focus
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn th(m: &Metrics) -> Theme {
        Theme {
            metrics: m.clone(),
            ..Theme::default()
        }
    }

    fn wide_metrics() -> Metrics {
        Metrics {
            menu_width: 180,
            startmenu_width: 232,
            ..Metrics::default()
        }
    }

    fn wide_theme() -> Theme {
        Theme {
            metrics: wide_metrics(),
            ..Theme::default()
        }
    }
    use crate::anim::{Unroll, UnrollFrom};
    use crate::interface::Interface;
    use crate::raster::{Canvas, Rect};
    use crate::text::TextRenderer;
    use crate::theme::{FontConfig, Metrics, Theme};

    impl MenuItem {
        fn label_for_test(&self) -> String {
            match self {
                MenuItem::Entry { label, .. } => label.clone(),
                MenuItem::Separator => String::new(),
            }
        }
    }

    fn items() -> Vec<MenuItem> {
        vec![
            MenuItem::entry("Terminal"),
            MenuItem::Separator,
            MenuItem::entry("Shut Down..."),
        ]
    }

    #[test]
    fn expand_label_fills_in_the_user() {
        assert_eq!(expand_label("&Log Off {user}...", "jan"), "&Log Off jan...");
        assert_eq!(expand_label("&Run...", "jan"), "&Run...");
    }

    #[test]
    fn context_style_uses_context_menu_metrics() {
        let m = wide_metrics();
        let plain = |label: &str| MenuItem::entry(label);
        let items = vec![plain("Open"), MenuItem::Separator, plain("Delete")];
        let style = MenuStyle::context_for(&wide_theme(), &items);
        let l = MenuLayout::compute_styled(&style, &items, usize::MAX);
        assert_eq!(l.rows[0].height, 17);
        assert_eq!(l.rows[1].height, 9);
        assert_eq!(l.rows[2].y, BORDER + 17 + 9);
        assert_eq!(l.text_x, 17);
        assert_eq!(l.height, 2 * BORDER + 17 + 9 + 17);
        let mut with_icon = items.clone();
        if let MenuItem::Entry { icon, .. } = &mut with_icon[0] {
            *icon = Some(Canvas::new(16, 16));
        }
        let style = MenuStyle::context_for(&wide_theme(), &with_icon);
        let l = MenuLayout::compute_styled(&style, &with_icon, usize::MAX);
        assert_eq!(l.rows[0].height, 19);
        assert_eq!(l.rows[1].height, 9);
        assert_eq!(l.text_x, 28);
        assert_eq!(l.icon_x, 6);
        assert_eq!(l.scaled(2).icon_x, 12);
        let mut with_glyph = items.clone();
        if let MenuItem::Entry { glyph, .. } = &mut with_glyph[2] {
            *glyph = Some(MenuGlyph::Close);
        }
        let l = MenuLayout::compute_styled(
            &MenuStyle::context_for(&wide_theme(), &with_glyph),
            &with_glyph,
            usize::MAX,
        );
        assert_eq!(l.rows[0].height, 19);
        assert_eq!(l.text_x, 22);
        assert_eq!(l.icon_x, 2);
        let sub = MenuLayout::compute_styled(&MenuStyle::submenu(&th(&m)), &items, usize::MAX);
        assert_eq!(sub.rows[0].height, m.menu_item_height);
        assert_eq!(sub.rows[1].height, SEPARATOR_H);
    }

    #[test]
    fn a_menu_level_names_its_rows_and_focuses_the_highlighted_one() {
        let items = vec![
            MenuItem::entry("Programs").submenu(true),
            MenuItem::Separator,
            MenuItem::entry("Run..."),
        ];
        let lay = MenuLayout::compute(&wide_metrics(), &items);
        let rect = Rect::new(10, 20, lay.width, lay.height);
        let mut t = Tree::window("", (400, 300));
        let (menu, rows, hot) = access_menu(
            &mut t,
            access::ROOT,
            0,
            &items,
            &lay,
            rect,
            Some(2),
            Some(0),
        );
        assert_eq!(rows[1], menu, "a separator row maps to its menu");
        assert_eq!(hot, Some(rows[2]), "the highlighted row has the focus");
        let run = t.get(rows[2]).unwrap();
        assert_eq!(run.label(), Some("Run..."));
        assert_eq!(
            run.bounds().unwrap().y0,
            f64::from(lay.rows[2].y + 20),
            "rows sit at the menu's corner"
        );
        let programs = t.get(rows[0]).unwrap();
        assert_eq!(programs.is_expanded(), Some(true), "its submenu is open");
    }

    #[test]
    fn access_key_is_the_mnemonic_or_first_letter() {
        let (label, mnemonic) = split_mnemonic("Sear&ch");
        let searched = MenuItem::entry(label).mnemonic(mnemonic);
        assert_eq!(searched.access_key(), Some('c'));
        assert_eq!(MenuItem::entry("Run...").access_key(), Some('r'));
        assert_eq!(MenuItem::entry("...").access_key(), None);
        assert_eq!(MenuItem::Separator.access_key(), None);
        let items = vec![
            MenuItem::entry("Programs"),
            MenuItem::Separator,
            MenuItem::entry("Printers"),
            searched,
        ];
        let rows = |key| rows_for_key(&items, key).collect::<Vec<_>>();
        assert_eq!(rows('P'), vec![0, 2]);
        assert_eq!(rows('c'), vec![3]);
        assert!(rows('z').is_empty());
    }

    #[test]
    fn step_selection_wraps_and_skips_separators() {
        let mut items = items(); // Terminal, ---, Shut Down...
        assert_eq!(step_selection(&items, None, 1, KeyStops::Enabled), Some(0));
        assert_eq!(step_selection(&items, None, -1, KeyStops::Enabled), Some(2));
        assert_eq!(
            step_selection(&items, Some(0), 1, KeyStops::Enabled),
            Some(2)
        );
        assert_eq!(
            step_selection(&items, Some(2), 1, KeyStops::Enabled),
            Some(0)
        );
        assert_eq!(
            step_selection(&items, Some(0), -1, KeyStops::Enabled),
            Some(2)
        );
        if let MenuItem::Entry { enabled, .. } = &mut items[2] {
            *enabled = false;
        }
        assert_eq!(
            step_selection(&items, Some(0), 1, KeyStops::Enabled),
            Some(0)
        );
        assert_eq!(
            step_selection(&[MenuItem::Separator], None, 1, KeyStops::Enabled),
            None
        );
    }

    #[test]
    fn relisted_rows_close_up_open_up_and_the_hover_keeps_its_entry() {
        let m = Metrics::default();
        let (a, b, c) = (
            MenuItem::entry("a.txt"),
            MenuItem::entry("b.txt"),
            MenuItem::entry("c.txt"),
        );
        let launch = MenuItem::entry("Launch");
        let mut items = vec![a.clone(), b.clone(), MenuItem::Separator, launch.clone()];
        let mut layout = MenuLayout::compute(&m, &items);
        let mut hovered = Some(1);
        let new = vec![c.clone(), a.clone(), b.clone(), MenuItem::Separator, launch];
        let column = layout.column_width;
        relist(
            &mut items,
            &mut layout,
            &mut hovered,
            &["a", "b", "-", "L"],
            &["c", "a", "b", "-", "L"],
            new.clone(),
            column,
        );
        assert_eq!(items, new);
        assert_eq!(layout, MenuLayout::compute(&m, &items));
        assert_eq!(hovered, Some(2), "b.txt kept the hover a row down");

        let column = layout.column_width;

        relist(
            &mut items,
            &mut layout,
            &mut hovered,
            &["c", "a", "b", "-", "L"],
            &["c", "a", "-", "L"],
            vec![c, a, MenuItem::Separator, MenuItem::entry("Launch")],
            column,
        );
        assert_eq!(layout, MenuLayout::compute(&m, &items));
        assert_eq!(hovered, Some(3), "b.txt went: Launch slid into its place");

        let column = layout.column_width;

        relist(
            &mut items,
            &mut layout,
            &mut hovered,
            &["c", "a", "-", "L"],
            &["E"],
            vec![MenuItem::entry("(Empty)").enabled(false)],
            column,
        );
        assert_eq!(layout, MenuLayout::compute(&m, &items));
        assert_eq!(hovered, None, "(Empty) takes no hover");
    }

    #[test]
    fn a_relisted_row_keeps_its_item_and_the_width_stays() {
        let m = Metrics::default();
        let style = MenuStyle::compact(&m);
        let wide = MenuStyle {
            width: style.width + 40,
            ..style
        };
        let iconed = MenuItem::entry("a.txt").icon(Some(Canvas::new(1, 1)));
        let mut items = vec![iconed.clone()];
        let mut layout = MenuLayout::compute_styled(&wide, &items, usize::MAX);
        let mut hovered = None;
        let column = layout.column_width;
        relist(
            &mut items,
            &mut layout,
            &mut hovered,
            &["a"],
            &["b", "a"],
            vec![MenuItem::entry("b.txt"), MenuItem::entry("a.txt")],
            column,
        );
        assert_eq!(items, [MenuItem::entry("b.txt"), iconed]);
        assert_eq!(
            layout,
            MenuLayout::compute_styled(&wide, &items, usize::MAX)
        );
    }

    #[test]
    fn a_relisted_longer_name_widens_the_menu_as_an_open_would() {
        let m = Metrics {
            menu_width: 180,
            ..wide_metrics()
        };
        let text = TextRenderer::load(&FontConfig::default());
        let style = MenuStyle::submenu(&th(&m));
        let short = vec![MenuItem::entry("a.txt")];
        let mut items = short.clone();
        let mut layout =
            MenuLayout::compute_styled(&style.fit(&text, &items, 11, 640), &items, usize::MAX);
        let mut hovered = None;
        let long = vec![
            MenuItem::entry(format!("{}.txt", "x".repeat(40))),
            MenuItem::entry("a.txt"),
        ];
        let column = layout.fit_column(&text, &long, 11, 640);
        relist(
            &mut items,
            &mut layout,
            &mut hovered,
            &["a"],
            &["x", "a"],
            long.clone(),
            column,
        );
        let opened =
            MenuLayout::compute_styled(&style.fit(&text, &long, 11, 640), &long, usize::MAX);
        assert!(
            opened.width > m.menu_width,
            "the long name needs a wider menu"
        );
        assert_eq!(layout, opened);

        let column = layout.fit_column(&text, &short, 11, 640);
        relist(
            &mut items,
            &mut layout,
            &mut hovered,
            &["x", "a"],
            &["a"],
            short,
            column,
        );
        assert_eq!(
            layout.width, opened.width,
            "a relist never narrows the menu"
        );
    }

    #[test]
    fn underline_sits_under_the_mnemonic_only_in_keyboard_mode() {
        let theme = wide_theme();
        let text = TextRenderer::load(&FontConfig::default());
        let (label, mnemonic) = split_mnemonic("Sear&ch");
        let items = vec![MenuItem::entry(label).mnemonic(mnemonic)];
        let l = MenuLayout::compute(&theme.metrics, &items);
        let size = theme.fonts.size_px;
        let r = l.rows[0];
        let text_rect = Rect::new(r.x + l.text_x, r.y, r.width - l.text_x, r.height);
        let baseline = text.baseline(text_rect, false, size).unwrap();
        let (dx, w) = text.span("Search", 4, false, size);
        assert!(w > 0);
        let fg = theme.colors.menu_text.argb();
        for keyboard in [false, true] {
            let mut c = Canvas::new(l.width as u32, l.height as u32);
            draw_menu(&mut c, &theme, &text, &l, &items, None, "", keyboard);
            let row: Vec<u32> = (0..w)
                .map(|i| c.get(text_rect.x + dx + i, baseline + 1))
                .collect();
            assert_eq!(
                row.iter().all(|&p| p == fg),
                keyboard,
                "keyboard={keyboard}"
            );
            assert_ne!(c.get(text_rect.x + dx - 1, baseline + 1), fg);
        }
    }

    #[test]
    fn underline_is_dropped_when_the_ellipsis_cuts_the_mnemonic() {
        let theme = wide_theme();
        let text = TextRenderer::load(&FontConfig::default());
        let label = "Averylongapplicationnamethatcannotfit";
        let underlined = |m: usize| {
            let items = vec![MenuItem::entry(label).mnemonic(Some(m))];
            let mut l = MenuLayout::compute(&theme.metrics, &items);
            l.width = 90;
            l.rows[0].width = 90 - 2 * BORDER;
            let draw = |keyboard| {
                let mut c = Canvas::new(l.width as u32, l.height as u32);
                draw_menu(&mut c, &theme, &text, &l, &items, None, "", keyboard);
                c
            };
            let (plain, keyed) = (draw(false), draw(true));
            (0..l.height).any(|y| (0..l.width).any(|x| plain.get(x, y) != keyed.get(x, y)))
        };
        let text_x = MenuLayout::compute(&theme.metrics, &[MenuItem::entry(label)]).text_x;
        let width = 90 - 2 * BORDER - text_x;
        let shown = text.ellipsize(label, width - ARROW_SPACE, false, theme.fonts.size_px);
        let kept = shown.chars().count() - 3;
        assert!(kept > 0);
        for m in 0..label.chars().count() {
            assert_eq!(underlined(m), m < kept, "access key at char {m}");
        }
    }

    /// A frame of `items` unrolling from `from` halfway in at scale `s`: the menu and its
    /// shadow moved by the frame's shift and cut to its clip, as the compositor's scene shows it.
    fn unrolled(theme: &Theme, items: &[MenuItem], from: UnrollFrom, s: i32) -> Canvas {
        let text = TextRenderer::load(&FontConfig::default());
        let mut theme = theme.clone();
        theme.metrics.menu_width = 120;
        let theme = &theme;
        let layout = MenuLayout::compute_styled(&MenuStyle::submenu(theme), items, 99);
        let reach = shadow_reach(theme);
        let size = (layout.width + reach, layout.height + reach);
        let frame = Unroll::at(from, size, 0.5);
        let (theme, layout) = (theme.scaled(s), layout.scaled(s));
        let mut menu = Canvas::new(layout.width as u32, layout.height as u32);
        draw_menu(&mut menu, &theme, &text, &layout, items, Some(0), "", false);
        let shadow = draw_menu_shadow(&theme, layout.width, layout.height, i32::MAX);
        let (dx, dy) = (frame.shift.0 * s, frame.shift.1 * s);
        let c = frame.clip;
        let clip = Rect::new(c.x * s + dx, c.y * s + dy, c.width * s, c.height * s);
        let mut out = Canvas::new((size.0 * s) as u32, (size.1 * s) as u32);
        out.blit_over_clipped(dx, dy, &shadow, clip);
        out.blit_over_clipped(dx, dy, &menu, clip);
        out
    }

    #[test]
    fn half_unrolled_menus_at_scale_1_and_2() {
        let items = [
            MenuItem::entry("").submenu(true),
            MenuItem::Separator,
            MenuItem::entry("").checked(true),
            MenuItem::entry(""),
        ];
        let glass = crate::interfaces::theme_with(
            crate::interface::Interface::Glass,
            Default::default(),
            &Default::default(),
        );
        let mut got = Vec::new();
        for (look, theme) in [("Classic", Theme::default()), ("Glass", glass)] {
            for from in [
                UnrollFrom::Top,
                UnrollFrom::Bottom,
                UnrollFrom::Left,
                UnrollFrom::Right,
            ] {
                for s in [1, 2] {
                    let c = unrolled(&theme, &items, from, s);
                    let clear = |x: i32, y: i32| c.get(x, y) >> 24 == 0;
                    let (w, h) = (c.width as i32, c.height as i32);
                    let (near, far) = match from {
                        UnrollFrom::Top => ((w / 4, 0), (w / 4, h - 1)),
                        UnrollFrom::Bottom => ((w / 4, h - 1), (w / 4, 0)),
                        UnrollFrom::Left => ((0, h / 4), (w - 1, h / 4)),
                        UnrollFrom::Right => ((w - 1, h / 4), (0, h / 4)),
                    };
                    assert!(
                        !clear(near.0, near.1),
                        "{look} {from:?} {s}: the near edge shows menu"
                    );
                    assert!(
                        clear(far.0, far.1),
                        "{look} {from:?} {s}: the far edge is still empty"
                    );
                    got.push((format!("{look} {from:?} {s}"), fnv(&c)));
                }
            }
        }
        crate::pins::check("menu_unroll", &got);
    }

    fn fnv(c: &Canvas) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for v in [c.width, c.height]
            .into_iter()
            .chain(c.pixels.iter().copied())
        {
            for b in v.to_le_bytes() {
                h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
            }
        }
        h
    }

    #[test]
    fn shadow_fades_outward_and_spares_the_menu() {
        let c = draw_shadow(20, 10, 4);
        assert_eq!((c.width, c.height), (24, 14));
        assert_eq!(c.get(5, 5), 0);
        assert_eq!(c.get(21, 1), 0);
        assert_eq!(c.get(1, 11), 0);
        let alpha = |x: i32, y: i32| c.get(x, y) >> 24;
        assert_eq!(alpha(20, 5), 128);
        assert_eq!(alpha(23, 5), 32);
        assert!(alpha(20, 5) > alpha(21, 5) && alpha(21, 5) > alpha(23, 5));
        assert_eq!(alpha(5, 10), 128);
        assert!(alpha(21, 11) < alpha(21, 5));
        assert_eq!(c.get(20, 5) & 0x00FF_FFFF, 0);
        assert_eq!(draw_shadow(20, 10, 0).get(19, 9), 0);
    }

    #[test]
    fn a_shadow_cut_at_a_floor_keeps_only_the_rows_above_it() {
        let full = draw_shadow(20, 10, 4);
        let c = draw_shadow_above(20, 10, 4, 10);
        assert_eq!((c.width, c.height), (24, 10), "cut at the menu's foot");
        for y in 0..10 {
            for x in 0..24 {
                assert_eq!(c.get(x, y), full.get(x, y), "({x}, {y}) as uncut");
            }
        }
        assert_eq!(
            draw_shadow_above(20, 10, 4, 99),
            full,
            "a floor past the shadow cuts nothing"
        );
    }

    #[test]
    fn a_150_percent_context_menu_grows_its_rows_around_its_labels() {
        let theme = Theme::default().with_text_size(150);
        let items = vec![
            MenuItem::entry("Open"),
            MenuItem::Separator,
            MenuItem::entry("Paste"),
        ];
        let style = MenuStyle::context_for(&theme, &items);
        let l = MenuLayout::compute_styled(&style, &items, usize::MAX);
        assert_eq!(l.rows[0].height, 24);
        assert_eq!(l.rows[1].height, 9, "separators keep their size");
        assert_eq!(l.height, 2 * BORDER + 24 + 9 + 24);
        if !crate::text::test_tahoma() {
            return;
        }
        let text = TextRenderer::load(&theme.fonts);
        let l = MenuLayout::compute_styled(&style.fit(&text, &items, 17, 400), &items, usize::MAX);
        let mut c = Canvas::new(l.width as u32, l.height as u32);
        draw_menu(&mut c, &theme, &text, &l, &items, None, "", false);
        let ink = theme.colors.menu_text.argb();
        for (i, want) in [(0, (6, 22)), (2, (6, 18))] {
            let r = l.rows[i];
            let rows: Vec<i32> = (r.y - 3..r.bottom() + 3)
                .filter(|&y| (r.x..r.right()).any(|x| c.get(x, y) == ink))
                .collect();
            let (top, bottom) = (rows[0] - r.y, rows[rows.len() - 1] - r.y);
            assert_eq!((top, bottom), want, "row {i}'s ink rows");
        }
    }

    #[test]
    fn disabled_text_is_etched_and_long_labels_get_an_ellipsis() {
        let theme = wide_theme();
        let text = TextRenderer::load(&FontConfig::default());
        let mut off = MenuItem::entry("Off");
        if let MenuItem::Entry { enabled, .. } = &mut off {
            *enabled = false;
        }
        let items = vec![off];
        let l = MenuLayout::compute(&theme.metrics, &items);
        let mut c = Canvas::new(l.width as u32, l.height as u32);
        draw_menu(&mut c, &theme, &text, &l, &items, None, "", false);
        let gray = theme.colors.gray_text.argb();
        let white = theme.colors.button_hilight.argb();
        let r = l.rows[0];
        let text_rect = Rect::new(r.x + l.text_x, r.y, r.width - l.text_x, r.height);
        let mut etched = false;
        let mut whites = 0;
        for y in text_rect.y..text_rect.bottom() {
            for x in text_rect.x..text_rect.right() {
                if c.get(x, y) == white {
                    whites += 1;
                }
                if c.get(x, y) == gray && c.get(x + 1, y + 1) == white {
                    etched = true;
                }
            }
        }
        assert!(etched && whites > 0);

        let long = MenuItem::entry("A very long application name that cannot fit");
        let items = vec![long];
        let mut l = MenuLayout::compute(&theme.metrics, &items);
        let narrow = 90;
        l.width = narrow;
        for r in &mut l.rows {
            r.width = narrow - 2 * BORDER;
        }
        let mut c = Canvas::new(l.width as u32, l.height as u32);
        draw_menu(&mut c, &theme, &text, &l, &items, None, "", false);
        let mut ref_c = Canvas::new(l.width as u32, l.height as u32);
        let r = l.rows[0];
        let text_rect = Rect::new(
            r.x + l.text_x,
            r.y,
            (r.width - l.text_x - ARROW_SPACE).max(0),
            r.height,
        );
        ref_c.fill_rect(ref_c.bounds(), theme.colors.menu);
        let shown = text.ellipsize(
            "A very long application name that cannot fit",
            text_rect.width,
            false,
            theme.fonts.size_px,
        );
        assert!(shown.ends_with("..."));
        text.draw(
            &mut ref_c,
            text_rect,
            &shown,
            false,
            theme.colors.menu_text,
            Align::Left,
            theme.fonts.size_px,
        );
        for y in text_rect.y..text_rect.bottom() {
            for x in text_rect.x..text_rect.right() {
                assert_eq!(c.get(x, y), ref_c.get(x, y), "at {x},{y}");
            }
        }
    }

    #[test]
    fn layout_rows() {
        let m = wide_metrics();
        let l = MenuLayout::compute(&m, &items());
        assert_eq!(l.width, 180);
        assert_eq!(l.rows[0], Rect::new(3, 3, 174, 20));
        assert_eq!(l.rows[1], Rect::new(3, 23, 174, 8));
        assert_eq!(l.rows[2], Rect::new(3, 31, 174, 20));
        assert_eq!(l.height, 3 + 20 + 8 + 20 + 3);
        assert_eq!(l.columns, 1);
    }

    #[test]
    fn compute_columns_wraps_by_count() {
        let m = Metrics {
            menu_width: 180,
            ..wide_metrics()
        };
        let entries: Vec<MenuItem> = (0..5).map(|i| MenuItem::entry(i.to_string())).collect();
        let l = MenuLayout::compute_columns(&m, &entries, 2);
        assert_eq!(l.columns, 3);
        assert_eq!(l.width, 3 * m.menu_width);
        assert_eq!(l.rows[2].x, BORDER + m.menu_width);
        assert_eq!(l.rows[3].x, BORDER + m.menu_width);
        assert_eq!(l.height, 2 * m.menu_item_height + 2 * BORDER);
    }

    #[test]
    fn compute_columns_counts_separators_as_rows() {
        // A separator still ends a column at max_rows, even though its own
        // height differs from an entry's.
        let m = Metrics {
            menu_width: 180,
            ..wide_metrics()
        };
        let items = vec![
            MenuItem::entry("a"),
            MenuItem::Separator,
            MenuItem::entry("b"),
        ];
        let l = MenuLayout::compute_columns(&m, &items, 2);
        assert_eq!(l.columns, 2);
        assert_eq!(l.rows[0].x, BORDER);
        assert_eq!(l.rows[1].x, BORDER);
        assert_eq!(l.rows[2].x, BORDER + m.menu_width);
    }

    #[test]
    fn submenu_max_rows_from_usable_height() {
        let m = wide_metrics();
        assert_eq!(submenu_max_rows(&m, 2 * BORDER + 3 * m.menu_item_height), 3);
        assert_eq!(submenu_max_rows(&m, 0), 1);
    }

    #[test]
    fn place_submenu_aligns_row_and_clamps() {
        let usable = Rect::new(0, 0, 800, 600);
        let parent = Rect::new(100, 50, 180, 200);
        let row = Rect::new(103, 90, 174, 20);
        let (x, y) = place_submenu(parent, row, (180, 100), usable);
        assert_eq!((x, y), (parent.right() - 2 * BORDER, row.y - 3));
        let (_, y) = place_submenu(parent, row, (180, 550), usable);
        assert_eq!(y, usable.bottom() - 550);
        let far_right = Rect::new(700, 50, 180, 200);
        let far_row = Rect::new(703, 90, 174, 20);
        let (x, _) = place_submenu(far_right, far_row, (180, 100), usable);
        assert_eq!(x, far_right.x - 180 + 2 * BORDER);
    }

    #[test]
    fn popup_opens_at_the_pointer_and_flips_up_over_the_taskbar() {
        let usable = Rect::new(0, 0, 1920, 1050);
        assert_eq!(place_popup((100, 100), (150, 116), usable), (100, 100));
        assert_eq!(place_popup((120, 1066), (150, 116), usable), (120, 934));
        assert_eq!(place_popup((1900, 100), (150, 116), usable), (1750, 100));
        assert_eq!(place_popup((10, 10), (150, 2000), usable), (10, 0));
    }

    #[test]
    fn a_child_menu_overlaps_its_parent_by_both_borders() {
        const PARENT_X: i32 = 68;
        const PARENT_W: i32 = 127;
        const CHILD_X: i32 = 189;
        const ROW_Y: i32 = 180;
        const CHILD_Y: i32 = 177;
        let usable = Rect::new(0, 0, 792, 573);
        let parent = Rect::new(PARENT_X, 22, PARENT_W, 238);
        let row = Rect::new(parent.x + BORDER, ROW_Y, PARENT_W - 2 * BORDER, 17);
        assert_eq!(
            place_submenu(parent, row, (118, 120), usable),
            (CHILD_X, CHILD_Y)
        );
        assert_eq!(CHILD_X + BORDER, parent.right() - BORDER);
    }

    #[test]
    fn place_submenu_pins_to_usable_left_when_flip_still_overflows() {
        // A submenu wider than the usable area: after flipping left of the
        // parent it would still start left of `usable.x`, so it pins there
        // instead of running off-screen.
        let usable = Rect::new(0, 0, 400, 600);
        let parent = Rect::new(0, 50, 180, 200);
        let row = Rect::new(3, 90, 174, 20);
        let wide_3col = (3 * 180, 300); // three columns, wider than `usable`
        let (x, _) = place_submenu(parent, row, wide_3col, usable);
        assert_eq!(x, usable.x);
    }

    #[test]
    fn open_submenu_wiring_aligns_first_entry_with_programs_row() {
        // Composes a root row layout, the row-to-absolute translation and
        // `place_submenu`; the row must be absolute, not menu-local.
        let m = wide_metrics();
        let root_items = vec![
            MenuItem::entry("Programs"),
            MenuItem::Separator,
            MenuItem::entry("Terminal"),
        ];
        let root_layout = MenuLayout::compute(&m, &root_items);
        let parent = Rect::new(0, 600, root_layout.width, root_layout.height);
        let local_row = root_layout.rows[0]; // the "Programs" row
        assert_eq!(local_row.y, BORDER); // the first row sits right under the border
        let row = Rect::new(
            parent.x + local_row.x,
            parent.y + local_row.y,
            local_row.width,
            local_row.height,
        );

        let apps: Vec<MenuItem> = (0..5).map(|i| MenuItem::entry(format!("app{i}"))).collect();
        let submenu_layout = MenuLayout::compute_columns(&m, &apps, submenu_max_rows(&m, 720));
        let usable = Rect::new(0, 0, 800, 720);
        let (_, y) = place_submenu(
            parent,
            row,
            (submenu_layout.width, submenu_layout.height),
            usable,
        );
        assert_eq!(y, 600 + BORDER - 3);
    }

    #[test]
    fn hit_skips_separators() {
        let m = wide_metrics();
        let l = MenuLayout::compute(&m, &items());
        assert_eq!(l.hit(50, 10), Some(0));
        assert_eq!(l.hit(50, 26), None);
        assert_eq!(l.hit(50, 40), Some(2));
        assert_eq!(l.hit(-1, 10), None);
        assert_eq!(l.hit(50, 200), None);
    }

    #[test]
    fn hit_uses_kind_not_height() {
        // menu_item_height equal to the separator height must not make
        // entries unclickable or the separator clickable.
        let m = Metrics {
            menu_item_height: 8,
            ..wide_metrics()
        };
        let l = MenuLayout::compute(&m, &items());
        assert_eq!(l.hit(50, l.rows[0].y + 2), Some(0));
        assert_eq!(l.hit(50, l.rows[1].y + 2), None);
    }

    #[test]
    fn compute_with_no_items() {
        let m = wide_metrics();
        let l = MenuLayout::compute(&m, &[]);
        assert_eq!(l.height, 2 * BORDER);
        assert!(l.rows.is_empty());
        assert_eq!(l.hit(5, 5), None);
    }

    #[test]
    fn hovered_row_uses_hilight() {
        let theme = wide_theme();
        let text = TextRenderer::load(&FontConfig::default());
        let l = MenuLayout::compute(&theme.metrics, &items());
        let mut c = Canvas::new(l.width as u32, l.height as u32);
        draw_menu(&mut c, &theme, &text, &l, &items(), Some(2), "", false);
        assert_eq!(c.get(5, l.rows[2].y + 2), theme.colors.hilight.argb());
        assert_eq!(c.get(5, l.rows[0].y + 2), theme.colors.menu.argb());
        assert_eq!(c.get(0, 10), theme.colors.button_face.argb());
        assert_eq!(c.get(1, 10), theme.colors.button_hilight.argb());
    }

    #[test]
    fn scaled_layout_doubles_rows() {
        let m = wide_metrics();
        let l = MenuLayout::compute(&m, &items());
        let s = l.scaled(2);
        assert_eq!(s.width, 360);
        assert_eq!(s.height, l.height * 2);
        assert_eq!(s.rows[0], Rect::new(6, 6, 348, 40));
        assert_eq!(s.selectable, l.selectable);
        assert_eq!(s.columns, 1);
    }

    #[test]
    fn hovered_row_at_scale_two() {
        let theme = wide_theme().scaled(2);
        let text = TextRenderer::load(&FontConfig::default());
        let l = MenuLayout::compute(&wide_theme().metrics, &items()).scaled(2);
        let mut c = Canvas::new(l.width as u32, l.height as u32);
        draw_menu(&mut c, &theme, &text, &l, &items(), Some(2), "", false);
        assert_eq!(c.get(10, l.rows[2].y + 4), theme.colors.hilight.argb());
        assert_eq!(c.get(10, l.rows[0].y + 4), theme.colors.menu.argb());
        assert_eq!(c.get(0, 20), theme.colors.button_face.argb());
        assert_eq!(c.get(1, 20), theme.colors.button_face.argb());
        assert_eq!(c.get(2, 20), theme.colors.button_hilight.argb());
        assert_eq!(c.get(3, 20), theme.colors.button_hilight.argb());
        let sep = l.rows[1];
        let y = sep.y + sep.height / 2 - 2;
        assert_eq!(c.get(40, y), theme.colors.button_shadow.argb());
        assert_eq!(c.get(40, y + 1), theme.colors.button_shadow.argb());
        assert_eq!(c.get(40, y + 2), theme.colors.button_hilight.argb());
    }

    #[test]
    fn root_style_reserves_band_and_tall_rows() {
        let m = Metrics {
            startmenu_width: 232,
            ..wide_metrics()
        };
        let style = MenuStyle::root(&th(&m));
        assert_eq!(style.icon_px(), 24);
        assert_eq!(MenuStyle::submenu(&th(&m)).icon_px(), 16);
        let l = MenuLayout::compute_styled(&style, &items(), usize::MAX);
        assert_eq!(l.width, m.startmenu_width);
        assert_eq!(l.band, m.startmenu_band_width);
        assert_eq!(
            l.rows[0],
            Rect::new(
                m.startmenu_band_width + BORDER,
                BORDER,
                m.startmenu_width - m.startmenu_band_width - 2 * BORDER,
                m.startmenu_item_height
            )
        );
        assert_eq!(l.text_x, ROOT_TEXT_X);
        let s = l.scaled(2);
        assert_eq!(s.band, 2 * m.startmenu_band_width);
        assert_eq!(s.text_x, 2 * ROOT_TEXT_X);
        assert_eq!(s.column_width, 2 * l.column_width);
    }

    #[test]
    fn fit_widens_for_long_labels_only() {
        let m = Metrics {
            menu_width: 180,
            ..wide_metrics()
        };
        let text = TextRenderer::load(&FontConfig::default());
        let style = MenuStyle::submenu(&th(&m));
        assert_eq!(style.fit(&text, &items(), 11, i32::MAX).width, m.menu_width);
        let long = vec![MenuItem::entry("x".repeat(80))];
        let wide = style.fit(&text, &long, 11, i32::MAX);
        let label_w = text.measure(&long[0].label_for_test(), false, 11);
        assert_eq!(wide.width, BORDER + TEXT_X + label_w + RIGHT_PAD + BORDER);
        assert!(wide.width > m.menu_width);
        let fitted = MenuStyle::submenu(&Theme::default()).fit(&text, &items(), 11, i32::MAX);
        let longest = items()
            .iter()
            .map(|it| match it {
                MenuItem::Entry { label, .. } => text.measure(label, false, 11),
                MenuItem::Separator => 0,
            })
            .max()
            .unwrap();
        assert_eq!(fitted.width, BORDER + TEXT_X + longest + RIGHT_PAD + BORDER);
        let root = MenuStyle::root(&Theme::default()).fit(&text, &items(), 11, i32::MAX);
        assert_eq!(
            root.width,
            Metrics::default().startmenu_band_width
                + BORDER
                + ROOT_TEXT_X
                + longest
                + ROOT_RIGHT_PAD
                + BORDER
        );
    }

    #[test]
    fn fit_stops_at_the_max_width_but_keeps_the_minimum() {
        let m = Metrics {
            menu_width: 180,
            ..wide_metrics()
        };
        let text = TextRenderer::load(&FontConfig::default());
        let style = MenuStyle::submenu(&th(&m));
        let long = vec![MenuItem::entry("x".repeat(1000))];
        assert_eq!(style.fit(&text, &long, 11, 640).width, 640);
        assert_eq!(style.fit(&text, &long, 11, 100).width, 180);
    }

    #[test]
    fn band_is_drawn_with_gradient_and_banner() {
        let theme = wide_theme();
        let text = TextRenderer::load(&FontConfig::default());
        let l = MenuLayout::compute_styled(&MenuStyle::root(&theme), &items(), usize::MAX);
        let mut c = Canvas::new(l.width as u32, l.height as u32);
        draw_menu(&mut c, &theme, &text, &l, &items(), None, "Reprise", false);
        let cs = &theme.colors;
        assert_eq!(
            c.get(BORDER + 1, l.height - BORDER - 1),
            cs.menu_band.argb()
        );
        assert_eq!(c.get(BORDER + 1, BORDER), cs.menu_band_top.argb());
        let third_up = l.height - BORDER - (l.height - 2 * BORDER) / 3;
        assert_eq!(c.get(BORDER + 1, third_up), cs.gradient_menu_band.argb());
        assert_eq!(c.get(l.rows[0].x, l.rows[0].y), cs.menu.argb());
        let white = cs.menu_band_text.argb();
        let has_text = (BORDER..BORDER + l.band)
            .any(|x| (BORDER..l.height - BORDER).any(|y| c.get(x, y) == white));
        assert!(has_text, "expected banner pixels in the band");
    }

    #[test]
    fn each_blue_schemes_start_band_pins_at_scale_1_and_2() {
        use crate::blue::Variant;
        let text = TextRenderer::load(&FontConfig::default());
        let mut got = Vec::new();
        for v in Variant::ALL {
            for d in [1, 2] {
                let theme = crate::interfaces::blue_theme(v).scaled(d);
                let style = MenuStyle {
                    width: 120 * d,
                    ..MenuStyle::root(&theme)
                };
                let l = MenuLayout::compute_styled(&style, &items(), usize::MAX);
                let mut c = Canvas::new(l.width as u32, l.height as u32);
                draw_menu(&mut c, &theme, &text, &l, &items(), None, "", false);
                let (inset, h) = (l.inset, l.height);
                let band = c.copy_rect(Rect::new(
                    inset.left,
                    inset.top,
                    l.band,
                    h - inset.top - inset.bottom,
                ));
                let hash = band.pixels.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &p| {
                    (h ^ u64::from(p)).wrapping_mul(0x0000_0100_0000_01b3)
                });
                got.push((format!("{v:?} {d}"), hash));
            }
        }
        crate::pins::check("blue/start_band", &got);
    }

    #[test]
    fn entry_constructor_defaults() {
        let e = MenuItem::entry("x");
        assert!(matches!(
            e,
            MenuItem::Entry { ref label, icon: None, enabled: true, checked: false, radio: false, submenu: false, mnemonic: None, shortcut: None, bold: false, glyph: None } if label == "x"
        ));
    }

    #[test]
    fn entry_icon_is_drawn_in_the_gutter() {
        let theme = wide_theme();
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.icon_size() as u32;
        let mut red = Canvas::new(px, px);
        red.fill_rect(red.bounds(), crate::theme::Color::rgb(255, 0, 0));
        let items = vec![MenuItem::entry("App").icon(Some(red))];
        let l = MenuLayout::compute(&theme.metrics, &items);
        let mut c = Canvas::new(l.width as u32, l.height as u32);
        draw_menu(&mut c, &theme, &text, &l, &items, None, "", false);
        let r = l.rows[0];
        assert_eq!(c.get(r.x + ICON_X + 8, r.y + r.height / 2), 0xFFFF_0000);
        assert_eq!(
            c.get(r.x + ICON_X + 16 + 2, r.y + r.height / 2),
            theme.colors.menu.argb()
        );
    }

    #[test]
    fn disabled_entry_uses_gray_text_even_when_hovered() {
        let theme = wide_theme();
        let text = TextRenderer::load(&FontConfig::default());
        let items = vec![MenuItem::entry("Off").enabled(false)];
        let l = MenuLayout::compute(&theme.metrics, &items);
        let mut c = Canvas::new(l.width as u32, l.height as u32);
        draw_menu(&mut c, &theme, &text, &l, &items, Some(0), "", false);
        let r = l.rows[0];
        assert_eq!(c.get(r.x + 2, r.y + 2), theme.colors.hilight.argb());
        let gray = theme.colors.gray_text.argb();
        let white = theme.colors.button_hilight.argb();
        let label = (r.x + TEXT_X..r.right()).flat_map(|x| (r.y..r.bottom()).map(move |y| (x, y)));
        let (mut has_gray, mut has_white) = (false, false);
        for (x, y) in label {
            has_gray |= c.get(x, y) == gray;
            has_white |= c.get(x, y) == white;
        }
        assert!(has_gray && !has_white, "gray {has_gray} white {has_white}");
        let mut c = Canvas::new(l.width as u32, l.height as u32);
        draw_menu(&mut c, &theme, &text, &l, &items, None, "", false);
        assert_eq!(c.get(r.x + 2, r.y + 2), theme.colors.menu.argb());
    }

    #[test]
    fn context_menus_fit_their_labels() {
        if !crate::text::test_tahoma() {
            return;
        }
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let fit = |items: &[MenuItem]| {
            let style = MenuStyle::context_for(&theme, items).fit(
                &text,
                items,
                theme.fonts.size_px,
                i32::MAX,
            );
            MenuLayout::compute_styled(&style, items, usize::MAX)
        };
        let e = MenuItem::entry;
        let view = [
            e("Toolbars").submenu(true),
            e("Status Bar").checked(true),
            e("Explorer Bar").submenu(true),
            MenuItem::Separator,
            e("Thumbnails"),
            e("Tiles").radio(true).checked(true),
            e("Icons"),
            e("List"),
            e("Details"),
            MenuItem::Separator,
            e("Arrange Icons by").submenu(true),
            MenuItem::Separator,
            e("Choose Details..."),
            e("Go To").submenu(true),
            e("Refresh"),
        ];
        assert_eq!(fit(&view).width, 127);
        let arrange = [
            e("Name"),
            e("Size"),
            e("Type"),
            e("Modified"),
            MenuItem::Separator,
            e("Show in Groups").enabled(false),
            e("Auto Arrange"),
            e("Align to Grid").checked(true),
            MenuItem::Separator,
            e("Show Desktop Icons").checked(true),
            e("Lock Web Items on Desktop"),
            e("Run Desktop Cleanup Wizard"),
        ];
        assert_eq!(fit(&arrange).width, 183);
        let icon = || Some(Canvas::new(16, 16));
        let new = [
            e("Folder").icon(icon()),
            e("Shortcut").icon(icon()),
            MenuItem::Separator,
            e("Text Document").icon(icon()),
            e("Compressed (zipped) Folder").icon(icon()),
        ];
        let l = fit(&new);
        assert_eq!((l.width, l.rows[0].height), (199, 19));
        let mut c = Canvas::new(l.width as u32, l.height as u32);
        draw_menu(&mut c, &theme, &text, &l, &new, None, "", false);
        let r = l.rows[0];
        let ink = theme.colors.menu_text.argb();
        let first = (l.icon_x + 16..r.width)
            .find(|&x| (0..r.height).any(|y| c.get(r.x + x, r.y + y) == ink));
        assert_eq!(first, Some(28), "Folder's first inked column");
    }

    #[test]
    fn a_context_rows_label_inks_17_px_in() {
        const EXPECTED_INK_X: i32 = 17;
        if !crate::text::test_tahoma() {
            return;
        }
        let theme = wide_theme();
        let text = TextRenderer::load(&FontConfig::default());
        for (label, top) in [("Properties", 4), ("New", 4), ("List", 4), ("Refresh", 3)] {
            let items = vec![MenuItem::entry(label)];
            let style = MenuStyle::context(&theme, ContextBitmaps::None);
            let l = MenuLayout::compute_styled(&style, &items, usize::MAX);
            let mut c = Canvas::new(l.width as u32, l.height as u32);
            draw_menu(&mut c, &theme, &text, &l, &items, None, "", false);
            let r = l.rows[0];
            let ink = theme.colors.menu_text.argb();
            let inked = |x: i32, y: i32| c.get(r.x + x, r.y + y) == ink;
            let first = (0..r.width).find(|&x| (0..r.height).any(|y| inked(x, y)));
            assert_eq!(first, Some(EXPECTED_INK_X), "{label}'s first inked column");
            let first_row = (0..r.height).find(|&y| (0..r.width).any(|x| inked(x, y)));
            assert_eq!(first_row, Some(top), "{label}'s first inked row");
        }
    }

    #[test]
    fn a_checked_row_inks_its_mark_4_px_in_and_6_down() {
        const EXPECTED_ROW_H: i32 = 17;
        const EXPECTED_INK: (i32, i32) = (4, 6);
        const EXPECTED_MARK: [&str; 7] = [
            "......X", ".....XX", "X...XXX", "XX.XXX.", "XXXXX..", ".XXX...", "..X....",
        ];
        let theme = wide_theme();
        let text = TextRenderer::load(&FontConfig::default());
        let items = vec![MenuItem::entry("Lock the Taskbar").checked(true)];
        let style = MenuStyle::context(&theme, ContextBitmaps::None);
        let l = MenuLayout::compute_styled(&style, &items, usize::MAX);
        let mut c = Canvas::new(l.width as u32, l.height as u32);
        draw_menu(&mut c, &theme, &text, &l, &items, None, "", false);
        let r = l.rows[0];
        assert_eq!(r.height, EXPECTED_ROW_H);
        let ink = theme.colors.menu_text.argb();
        for y in 0..r.height {
            for x in 0..l.text_x - 2 {
                let (gx, gy) = (x - EXPECTED_INK.0, y - EXPECTED_INK.1);
                let want = (0..7).contains(&gx)
                    && (0..7).contains(&gy)
                    && EXPECTED_MARK[gy as usize].as_bytes()[gx as usize] == b'X';
                assert_eq!(c.get(r.x + x, r.y + y) == ink, want, "row pixel ({x}, {y})");
            }
        }
    }

    #[test]
    fn a_radio_entry_draws_a_bullet_instead_of_a_check() {
        const ROW_X: i32 = 163;
        const ROW_Y: i32 = 213;
        const ROW_H: i32 = 17;
        const EXPECTED_BULLET_X: i32 = 168;
        const EXPECTED_BULLET_Y: i32 = 218;
        const EXPECTED_BULLET_D: i32 = 6;
        const CHECK_ROW_X: i32 = 284;
        const EXPECTED_CHECK_X: i32 = 288;
        assert_eq!(
            BULLET_X - CHECK_X,
            (EXPECTED_BULLET_X - ROW_X) - (EXPECTED_CHECK_X - CHECK_ROW_X)
        );
        let theme = wide_theme();
        let text = TextRenderer::load(&FontConfig::default());
        let items = vec![MenuItem::entry("Tiles").checked(true).radio(true)];
        let style = MenuStyle::context(&theme, ContextBitmaps::None);
        let l = MenuLayout::compute_styled(&style, &items, usize::MAX);
        let mut c = Canvas::new(l.width as u32, l.height as u32);
        draw_menu(&mut c, &theme, &text, &l, &items, None, "", false);
        let r = l.rows[0];
        assert_eq!(r.height, ROW_H, "a context menu's rows");
        let ink = theme.colors.menu_text.argb();
        let gutter = Rect::new(r.x, r.y, l.text_x, r.height);
        let cols: Vec<i32> = (gutter.x..gutter.right())
            .filter(|&x| (gutter.y..gutter.bottom()).any(|y| c.get(x, y) == ink))
            .collect();
        let rows: Vec<i32> = (gutter.y..gutter.bottom())
            .filter(|&y| (gutter.x..gutter.right()).any(|x| c.get(x, y) == ink))
            .collect();
        assert_eq!(cols.len(), EXPECTED_BULLET_D as usize);
        assert_eq!(rows.len(), EXPECTED_BULLET_D as usize);
        assert_eq!(cols[0] - r.x, BULLET_X);
        assert_eq!(rows[0] - r.y, EXPECTED_BULLET_Y - ROW_Y);
        assert_eq!(c.get(cols[0], rows[0]), theme.colors.menu.argb());
        assert_eq!(c.get(cols[1], rows[0]), ink);
        for x in &cols {
            assert_eq!(c.get(*x, rows[2]), ink);
        }
        let (gx, gy) = (r.x + CHECK_X, r.y + (r.height - 7) / 2);
        assert_eq!(c.get(gx + 6, gy), theme.colors.menu.argb());
    }

    #[test]
    #[ignore]
    fn render_menu_radio() {
        let theme = wide_theme();
        let text = TextRenderer::load(&FontConfig::default());
        let row = |label: &str, checked: bool, radio: bool| {
            MenuItem::entry(label).checked(checked).radio(radio)
        };
        let items = vec![
            row("Thumbnails", false, true),
            row("Tiles", true, true),
            row("Icons", false, true),
            MenuItem::Separator,
            row("Status Bar", true, false),
        ];
        let style = MenuStyle::context(&theme, ContextBitmaps::None);
        let l = MenuLayout::compute_styled(&style, &items, usize::MAX);
        let mut c = Canvas::new(l.width as u32, l.height as u32);
        draw_menu(&mut c, &theme, &text, &l, &items, Some(1), "", false);
        crate::raster::write_preview(&c, "menu-radio-preview.png");
    }

    fn nested_tree() -> MenuTree {
        let arrange = MenuTree::flat(vec![
            MenuItem::item("&Name"),
            MenuItem::item("Si&ze"),
            MenuItem::item("T&ype"),
        ]);
        let mut t = MenuTree::new();
        t.push(MenuItem::item("Status &Bar"));
        t.push_sub(MenuItem::item("Arrange &Icons by"), arrange);
        t.push(MenuItem::Separator);
        t.push(MenuItem::item("R&efresh"));
        t
    }

    fn nested_ctx<'a>(theme: &'a Theme, text: &'a TextRenderer) -> MenuCtx<'a> {
        MenuCtx {
            text,
            theme,
            px: theme.fonts.size_px,
            within: Rect::new(0, 0, 800, 600),
        }
    }

    fn row_point(p: &PopupMenu, lvl: usize, i: usize) -> (i32, i32) {
        let level = &p.levels()[lvl];
        let r = level.layout.rows[i];
        (
            level.origin.0 + r.x + 5,
            level.origin.1 + r.y + r.height / 2,
        )
    }

    /// The nested chain with its submenu open, from the keyboard.
    fn opened_nested(t: &MenuTree, cx: &MenuCtx) -> PopupMenu {
        let mut p = PopupMenu::open(t, (100, 50), cx, true);
        p.key(t, MenuKey::Down, cx);
        p.key(t, MenuKey::Right, cx);
        assert_eq!(p.depth(), 2);
        p
    }

    #[test]
    fn levels_drawn_alone_and_stacked_make_the_whole_chain() {
        let text = TextRenderer::load(&FontConfig::default());
        for interface in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let base = crate::interfaces::theme_for(interface);
            let cx = nested_ctx(&base, &text);
            let t = nested_tree();
            let p = opened_nested(&t, &cx);
            let m = level_shadow(&base);
            for s in [1, 2] {
                let theme = base.scaled(s);
                let mut whole = Canvas::new(800 * s as u32, 600 * s as u32);
                p.draw(&mut whole, &theme, &text, false);
                let mut stacked = Canvas::new(800 * s as u32, 600 * s as u32);
                for (i, level) in p.levels().iter().enumerate() {
                    let r = level.rect();
                    let mut own =
                        Canvas::new(((r.width + m) * s) as u32, ((r.height + m) * s) as u32);
                    p.draw_level(i, (-r.x, -r.y), &mut own, &theme, &text, false);
                    stacked.blit_over(r.x * s, r.y * s, &own);
                }
                let off = whole
                    .pixels
                    .iter()
                    .zip(&stacked.pixels)
                    .filter(|(a, b)| {
                        a.to_le_bytes()
                            .iter()
                            .zip(b.to_le_bytes())
                            .any(|(x, y)| x.abs_diff(y) > 1)
                    })
                    .count();
                assert_eq!(
                    off, 0,
                    "{interface:?} at {s}: no pixel off by more than a rounding step"
                );
            }
        }
    }

    #[test]
    fn a_level_shadow_reaches_as_far_as_the_look_casts_it() {
        assert_eq!(
            level_shadow(&Theme::default()),
            0,
            "Classic chains cast none"
        );
        let glass = crate::interfaces::theme_for(Interface::Glass);
        assert_eq!(level_shadow(&glass), crate::style::DropShadow::N as i32);
    }

    #[test]
    fn a_placed_level_is_hit_where_it_was_put() {
        let theme = wide_theme();
        let text = TextRenderer::load(&FontConfig::default());
        let cx = nested_ctx(&theme, &text);
        let t = nested_tree();
        let mut p = opened_nested(&t, &cx);
        let before = row_point(&p, 1, 0);
        p.place(1, (-300, 900));
        assert_eq!(p.levels()[1].origin, (-300, 900));
        assert_ne!(p.hit(before.0, before.1), Some((1, Some(0))));
        let (x, y) = row_point(&p, 1, 0);
        assert_eq!(
            p.hit(x, y),
            Some((1, Some(0))),
            "the moved row takes the point"
        );
        assert_eq!(
            p.click(&t, x, y, &cx),
            MenuAction::Chosen(vec![1, 0]),
            "a press on it picks it"
        );
    }

    #[test]
    fn a_submenu_anchor_puts_the_child_where_place_submenu_does() {
        let theme = wide_theme();
        let text = TextRenderer::load(&FontConfig::default());
        let cx = nested_ctx(&theme, &text);
        let t = nested_tree();
        let mut p = opened_nested(&t, &cx);
        p.place(0, (400, 50));
        assert_eq!(p.submenu_anchor(0), None, "the root opens at its point");
        let (anchor, dy) = p.submenu_anchor(1).expect("the submenu's anchor");
        let (parent, child) = (p.levels()[0].rect(), p.levels()[1].rect());
        let row = p.levels()[0].layout.rows[1];
        let row_abs = Rect::new(parent.x + row.x, parent.y + row.y, row.width, row.height);
        let size = (child.width, child.height);
        // Anchored on the rect's right end, growing right and down; flipped, on its left end
        // growing left.
        let right = (parent.x + anchor.right(), parent.y + anchor.y + dy);
        let left = (parent.x + anchor.x - child.width, right.1);
        let open = Rect::new(0, 0, 800, 600);
        assert_eq!(right, place_submenu(parent, row_abs, size, open));
        let tight = Rect::new(0, 0, parent.right() + 10, 600);
        assert_eq!(left, place_submenu(parent, row_abs, size, tight));
    }

    #[test]
    fn a_chain_opened_from_the_keyboard_underlines_without_the_hosts_cue() {
        let theme = wide_theme();
        let text = TextRenderer::load(&FontConfig::default());
        let cx = nested_ctx(&theme, &text);
        let t = nested_tree();
        let drawn = |p: &PopupMenu, cue: bool| {
            let mut c = Canvas::new(800, 600);
            p.draw(&mut c, &theme, &text, cue);
            c.pixels
        };
        let by_pointer = PopupMenu::open(&t, (100, 50), &cx, false);
        assert!(
            drawn(&by_pointer, false) != drawn(&by_pointer, true),
            "pointer: the cue decides"
        );
        let by_key = PopupMenu::open(&t, (100, 50), &cx, true);
        assert!(
            drawn(&by_key, false) == drawn(&by_key, true),
            "keyboard: underlined"
        );
    }

    #[test]
    fn a_submenu_opens_on_right_and_closes_on_left() {
        let theme = wide_theme();
        let text = TextRenderer::load(&FontConfig::default());
        let cx = nested_ctx(&theme, &text);
        let t = nested_tree();
        let mut p = PopupMenu::open(&t, (100, 50), &cx, true);
        assert_eq!((p.depth(), p.hovered()), (1, Some(0)));
        assert_eq!(p.key(&t, MenuKey::Right, &cx), MenuAction::None);
        p.key(&t, MenuKey::Down, &cx);
        assert_eq!(p.hovered(), Some(1));
        assert_eq!(p.key(&t, MenuKey::Right, &cx), MenuAction::Redraw);
        assert_eq!(p.depth(), 2);
        assert_eq!(p.hovered(), Some(0), "the child's first row is picked");
        let parent = p.levels()[0].rect();
        let r = p.levels()[0].layout.rows[1];
        let row = Rect::new(parent.x + r.x, parent.y + r.y, r.width, r.height);
        let child = p.levels()[1].rect();
        assert_eq!(
            (child.x, child.y),
            place_submenu(parent, row, (child.width, child.height), cx.within)
        );
        assert_eq!(child.x + BORDER, parent.right() - BORDER);
        p.key(&t, MenuKey::Down, &cx);
        assert_eq!(
            p.key(&t, MenuKey::Enter, &cx),
            MenuAction::Chosen(vec![1, 1])
        );
        let mut p = PopupMenu::open(&t, (100, 50), &cx, true);
        p.key(&t, MenuKey::Down, &cx);
        p.key(&t, MenuKey::Right, &cx);
        assert_eq!(p.key(&t, MenuKey::Left, &cx), MenuAction::Redraw);
        assert_eq!(p.depth(), 1);
        assert_eq!(p.key(&t, MenuKey::Left, &cx), MenuAction::None);
        assert_eq!(p.key(&t, MenuKey::Escape, &cx), MenuAction::Close);
        let mut p = PopupMenu::open(&t, (100, 50), &cx, true);
        assert_eq!(p.key(&t, MenuKey::Char('i'), &cx), MenuAction::Redraw);
        assert_eq!(p.depth(), 2);
        assert_eq!(
            p.key(&t, MenuKey::Char('z'), &cx),
            MenuAction::Chosen(vec![1, 1])
        );
    }

    fn drop_rows(default_enabled: bool) -> Vec<MenuItem> {
        vec![
            MenuItem::item("&Copy Here"),
            MenuItem::item("&Move Here")
                .bold(true)
                .enabled(default_enabled),
            MenuItem::item("Create &Shortcuts Here"),
            MenuItem::Separator,
            MenuItem::item("Cancel"),
        ]
    }

    #[test]
    fn enter_takes_the_highlighted_row_else_the_bold_one() {
        assert_eq!(default_row(&drop_rows(true)), Some(1));
        assert_eq!(enter_row(&drop_rows(true), None), Some(1));
        assert_eq!(enter_row(&drop_rows(true), Some(4)), Some(4));
        assert_eq!(default_row(&drop_rows(false)), None, "a greyed default");
        assert_eq!(enter_row(&drop_rows(false), None), None);
        assert_eq!(enter_row(&items(), None), None, "no bold row");

        let theme = wide_theme();
        let text = TextRenderer::load(&FontConfig::default());
        let cx = nested_ctx(&theme, &text);
        let t = MenuTree::flat(drop_rows(true));
        let mut p = PopupMenu::open(&t, (100, 50), &cx, false);
        assert_eq!(p.key(&t, MenuKey::Enter, &cx), MenuAction::Chosen(vec![1]));
        assert_eq!(p.hovered(), Some(1), "the default row lights up");
        let mut p = PopupMenu::open(&t, (100, 50), &cx, false);
        p.key(&t, MenuKey::Up, &cx);
        assert_eq!(p.key(&t, MenuKey::Enter, &cx), MenuAction::Chosen(vec![4]));
        let t = MenuTree::flat(drop_rows(false));
        let mut p = PopupMenu::open(&t, (100, 50), &cx, false);
        assert_eq!(p.key(&t, MenuKey::Enter, &cx), MenuAction::Redraw);
        assert_eq!(p.hovered(), None);
    }

    #[test]
    fn glass_arrow_keys_light_greyed_rows_and_enter_on_one_does_nothing() {
        use crate::interface::Interface;
        let edit = || {
            vec![
                MenuItem::item("&Undo").enabled(false),
                MenuItem::Separator,
                MenuItem::item("Cu&t").enabled(false),
                MenuItem::item("&Paste"),
            ]
        };
        let text = TextRenderer::load(&FontConfig::default());
        let t = MenuTree::flat(edit());
        let glass = crate::interfaces::theme_for(Interface::Glass);
        assert_eq!(KeyStops::of(&glass), KeyStops::Entries);
        let cx = nested_ctx(&glass, &text);
        let mut p = PopupMenu::open(&t, (100, 50), &cx, true);
        assert_eq!(
            p.hovered(),
            Some(0),
            "opened from the keys on a greyed Undo"
        );
        assert_eq!(p.key(&t, MenuKey::Enter, &cx), MenuAction::None);
        assert_eq!(p.depth(), 1, "the menu stays open");
        p.key(&t, MenuKey::Down, &cx);
        assert_eq!(
            p.hovered(),
            Some(2),
            "Down passes the separator onto a greyed Cut"
        );
        p.key(&t, MenuKey::Down, &cx);
        assert_eq!(p.key(&t, MenuKey::Enter, &cx), MenuAction::Chosen(vec![3]));
        let (x, y) = row_point(&p, 0, 2);
        assert_eq!(
            p.click(&t, x, y, &cx),
            MenuAction::None,
            "a click on a greyed row"
        );

        for interface in [Interface::Classic, Interface::Blue] {
            let theme = crate::interfaces::theme_for(interface);
            assert_eq!(KeyStops::of(&theme), KeyStops::Enabled, "{interface:?}");
            let cx = nested_ctx(&theme, &text);
            let mut p = PopupMenu::open(&t, (100, 50), &cx, true);
            assert_eq!(p.hovered(), Some(3), "{interface:?} opens on Paste");
            p.key(&t, MenuKey::Up, &cx);
            assert_eq!(p.hovered(), Some(3), "{interface:?} skips the greyed rows");
        }
    }

    #[test]
    fn enter_opens_a_bold_submenu_row() {
        let theme = wide_theme();
        let text = TextRenderer::load(&FontConfig::default());
        let cx = nested_ctx(&theme, &text);
        let mut t = nested_tree();
        t[1] = t[1].clone().bold(true);
        let mut p = PopupMenu::open(&t, (100, 50), &cx, false);
        assert_eq!(p.key(&t, MenuKey::Enter, &cx), MenuAction::Redraw);
        assert_eq!(p.depth(), 2);
        assert_eq!(p.levels()[0].hovered, Some(1));
        assert_eq!(p.hovered(), Some(0));
    }

    #[test]
    fn hovering_a_submenu_row_opens_it_and_leaving_it_closes_it() {
        let theme = wide_theme();
        let text = TextRenderer::load(&FontConfig::default());
        let cx = nested_ctx(&theme, &text);
        let t = nested_tree();
        let mut p = PopupMenu::open(&t, (100, 50), &cx, false);
        let (x, y) = row_point(&p, 0, 1);
        assert!(p.motion(&t, x, y, &cx));
        assert_eq!(p.depth(), 2, "hovering the row opens its child");
        let (cxp, cyp) = row_point(&p, 1, 2);
        assert!(p.motion(&t, cxp, cyp, &cx));
        assert_eq!(p.levels()[0].hovered, Some(1));
        assert_eq!(p.levels()[1].hovered, Some(2));
        assert!(!p.motion(&t, cxp, cyp, &cx));
        assert_eq!(p.depth(), 2);
        assert_eq!(p.click(&t, cxp, cyp, &cx), MenuAction::Chosen(vec![1, 2]));
        let (px0, py0) = row_point(&p, 0, 0);
        assert!(p.motion(&t, px0, py0, &cx));
        assert_eq!(p.depth(), 1);
        assert_eq!(p.click(&t, 700, 560, &cx), MenuAction::Close);
    }

    #[test]
    #[ignore]
    fn render_nested_menu() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let arrange = MenuTree::flat(vec![
            MenuItem::item("&Name"),
            MenuItem::item("Si&ze"),
            MenuItem::item("T&ype"),
            MenuItem::item("Modified"),
        ]);
        let mut view = MenuTree::new();
        view.push(MenuItem::item("&Toolbars"));
        view.push(MenuItem::item("Status &Bar"));
        view.push(MenuItem::Separator);
        view.push_sub(MenuItem::item("Arrange &Icons by"), arrange);
        view.push(MenuItem::Separator);
        view.push(MenuItem::item("R&efresh"));
        let cx = MenuCtx {
            text: &text,
            theme: &theme,
            px: theme.fonts.size_px,
            within: Rect::new(0, 0, 400, 300),
        };
        let mut p = PopupMenu::open(&view, (10, 10), &cx, false);
        p.key(&view, MenuKey::Down, &cx);
        p.key(&view, MenuKey::Down, &cx);
        p.key(&view, MenuKey::Down, &cx);
        p.key(&view, MenuKey::Right, &cx);
        let mut c = Canvas::new(400, 300);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        p.draw(&mut c, &theme, &text, true);
        crate::raster::write_preview(&c, "menu-nested-preview.png");
    }

    #[test]
    fn popup_levels_are_scaled_to_the_theme() {
        let theme = wide_theme().scaled(2);
        let text = TextRenderer::load(&FontConfig::default());
        let mut view = MenuTree::new();
        view.push(MenuItem::item("&Open"));
        view.push(MenuItem::item("E&xit"));
        let cx = MenuCtx {
            text: &text,
            theme: &wide_theme(),
            px: wide_theme().fonts.size_px,
            within: Rect::new(0, 0, 400, 300),
        };
        let p = PopupMenu::open(&view, (10, 10), &cx, false);
        let level = p.levels[0].rect();
        let fill = crate::theme::Color::rgb(1, 2, 3);
        let mut c = Canvas::new(800, 600);
        c.fill_rect(c.bounds(), fill);
        p.draw(&mut c, &theme, &text, false);
        let (far_x, far_y) = (level.right() * 2 - 1, level.bottom() * 2 - 1);
        assert_ne!(
            c.get(far_x, far_y),
            fill.argb(),
            "frame reaches the scaled edge"
        );
        assert_eq!(c.get(far_x + 1, far_y), fill.argb(), "and stops there");
        assert_eq!(
            c.get(20, 20),
            theme.colors.menu.argb(),
            "origin is scaled too"
        );
        assert_eq!(c.get(12, 12), fill.argb());
    }

    #[test]
    fn draw_menu_at_matches_a_blit_of_draw_menu() {
        let theme = wide_theme();
        let text = TextRenderer::load(&FontConfig::default());
        let items = vec![
            MenuItem::item("&Open"),
            MenuItem::Separator,
            MenuItem::item("Pr&operties"),
        ];
        let l = MenuLayout::compute(&theme.metrics, &items);
        let mut local = Canvas::new(l.width as u32, l.height as u32);
        draw_menu(&mut local, &theme, &text, &l, &items, Some(2), "", true);
        let mut copied = Canvas::new(200, 120);
        copied.blit_canvas(13, 7, &local);
        let mut direct = Canvas::new(200, 120);
        draw_menu_at(
            &mut direct,
            (13, 7),
            &theme,
            &text,
            &l,
            &items,
            Some(2),
            "",
            true,
        );
        assert_eq!(direct.pixels, copied.pixels);
    }

    #[test]
    fn submenu_entry_draws_right_arrow() {
        let theme = wide_theme();
        let text = TextRenderer::load(&FontConfig::default());
        let items = vec![MenuItem::entry("More").submenu(true)];
        let l = MenuLayout::compute(&theme.metrics, &items);
        let mut c = Canvas::new(l.width as u32, l.height as u32);
        draw_menu(&mut c, &theme, &text, &l, &items, None, "", false);
        let r = l.rows[0];
        let ax = r.right() - ARROW_RIGHT_INSET;
        let ay = r.y + (r.height - 7) / 2;
        assert_eq!(c.get(ax, ay), theme.colors.menu_text.argb());
        assert_eq!(c.get(ax, ay + 6), theme.colors.menu_text.argb());
        assert_eq!(c.get(ax + 3, ay + 3), theme.colors.menu_text.argb());
        assert_eq!(c.get(ax + 3, ay + 2), theme.colors.menu.argb());
    }

    fn flat_theme(v: crate::blue::Variant) -> Theme {
        let mut t = crate::interfaces::blue_theme(v);
        t.colors.menu_hilight = crate::theme::Color::rgb(0xBB, 0xB7, 0xC7);
        t.colors.hilight = crate::theme::Color::rgb(0xB2, 0xB4, 0xBF);
        t.colors.button_hilight = crate::theme::Color::rgb(0xF0, 0x10, 0x10);
        t
    }

    fn chrome_rows() -> Vec<MenuItem> {
        vec![
            MenuItem::entry(""),
            MenuItem::entry("").checked(true),
            MenuItem::entry("").checked(true).radio(true),
            MenuItem::Separator,
            MenuItem::entry("").enabled(false),
            MenuItem::entry("").submenu(true),
        ]
    }

    #[test]
    fn a_flat_menu_has_a_one_px_border_a_one_line_separator_and_a_framed_lit_row() {
        let text = TextRenderer::load(&FontConfig::default());
        for v in crate::blue::Variant::ALL {
            let theme = flat_theme(v);
            let cs = &theme.colors;
            let items = vec![
                MenuItem::item("&Toolbars"),
                MenuItem::item("Status &Bar"),
                MenuItem::Separator,
                MenuItem::item("&Details"),
            ];
            let style = MenuStyle::context_for(&theme, &items).fit(&text, &items, 11, 400);
            let l = MenuLayout::compute_styled(&style, &items, usize::MAX);
            let mut c = Canvas::new(l.width as u32, l.height as u32);
            draw_menu(&mut c, &theme, &text, &l, &items, Some(0), "", false);
            let (w, h) = (l.width, l.height);
            for (x, y) in [
                (0, 0),
                (w - 1, 0),
                (0, h - 1),
                (w - 1, h - 1),
                (0, h / 2),
                (w / 2, 0),
            ] {
                assert_eq!(
                    c.get(x, y),
                    cs.button_shadow.argb(),
                    "{v:?}: border at {x},{y}"
                );
            }
            for (x, y) in [(1, 1), (2, 2), (w - 2, h - 2), (w - 3, h / 2)] {
                assert_eq!(
                    c.get(x, y),
                    cs.menu.argb(),
                    "{v:?}: menu inside the border at {x},{y}"
                );
            }
            let lit = l.rows[0];
            assert_eq!((lit.x, lit.y), (3, 3), "{v:?}: the rows start 3 px in");
            for (x, y) in [(lit.x, lit.y), (lit.right() - 1, lit.bottom() - 1)] {
                assert_eq!(
                    c.get(x, y),
                    cs.hilight.argb(),
                    "{v:?}: the lit row's frame at {x},{y}"
                );
            }
            assert_eq!(
                c.get(lit.x + 1, lit.y + 1),
                cs.menu_hilight.argb(),
                "{v:?}: its fill"
            );
            let sep = l.rows[2];
            assert_eq!(sep.height, 9);
            let y = sep.y + 3;
            assert_eq!(
                c.get(sep.x, y),
                cs.menu.argb(),
                "{v:?}: the line starts a pixel in"
            );
            assert_eq!(
                c.get(sep.x + 1, y),
                cs.button_shadow.argb(),
                "{v:?}: the separator line"
            );
            assert_eq!(
                c.get(sep.right() - 2, y),
                cs.button_shadow.argb(),
                "{v:?}: its last pixel"
            );
            assert_eq!(
                c.get(sep.right() - 1, y),
                cs.menu.argb(),
                "{v:?}: a pixel short"
            );
            for dy in [-1, 1] {
                assert_eq!(
                    c.get(sep.x + 5, y + dy),
                    cs.menu.argb(),
                    "{v:?}: one line, no etch"
                );
            }
        }
    }

    #[test]
    fn a_flat_menus_greyed_label_has_no_etched_copy() {
        if !crate::text::test_tahoma() {
            return;
        }
        let theme = flat_theme(crate::blue::Variant::Blue);
        let text = TextRenderer::load(&FontConfig::default());
        let items = vec![
            MenuItem::item("&Delete")
                .enabled(false)
                .shortcut(Some("Del".into())),
        ];
        let style = MenuStyle::context_for(&theme, &items).fit(&text, &items, 11, 400);
        let l = MenuLayout::compute_styled(&style, &items, usize::MAX);
        let mut c = Canvas::new(l.width as u32, l.height as u32);
        draw_menu(&mut c, &theme, &text, &l, &items, None, "", false);
        let px = |col: crate::theme::Color| {
            (0..l.width)
                .flat_map(|x| (0..l.height).map(move |y| (x, y)))
                .filter(|&(x, y)| c.get(x, y) == col.argb())
                .count()
        };
        assert!(px(theme.colors.gray_text) > 20, "the label inks gray_text");
        assert_eq!(px(theme.colors.button_hilight), 0, "no etched copy");
    }

    #[test]
    fn blues_start_menu_rows_sit_inside_a_1_px_border() {
        let theme = crate::interfaces::blue_theme(crate::blue::Variant::Blue);
        let text = TextRenderer::load(&FontConfig::default());
        let items = vec![
            MenuItem::entry("").submenu(true),
            MenuItem::Separator,
            MenuItem::entry(""),
        ];
        let style = MenuStyle {
            width: 194,
            ..MenuStyle::root(&theme)
        };
        let l = MenuLayout::compute_styled(&style, &items, usize::MAX);
        assert_eq!(l.rows[0], Rect::new(22, 2, 171, 32));
        assert_eq!(l.rows[1], Rect::new(22, 34, 171, 8));
        assert_eq!(l.height, 2 + 32 + 8 + 32 + 1);
        let mut c = Canvas::new(l.width as u32, l.height as u32);
        draw_menu(&mut c, &theme, &text, &l, &items, None, "", false);
        let cs = &theme.colors;
        assert_eq!(c.get(0, 5), cs.button_shadow.argb(), "the border");
        assert_eq!(c.get(1, 1), cs.menu_band_top.argb(), "the band's top-left");
        assert_eq!(
            c.get(1, l.height - 2),
            cs.menu_band.argb(),
            "the band's bottom-left"
        );
        assert_eq!(c.get(100, 1), cs.menu.argb(), "the row of menu on top");
        let line = |x| c.get(x, 37) == cs.button_shadow.argb();
        assert!(
            !line(23) && line(24) && line(190) && !line(191),
            "separator"
        );
        let ink = |x| (2..34).any(|y| c.get(x, y) == cs.menu_text.argb());
        assert!(!ink(183) && ink(184) && ink(187) && !ink(188), "the arrow");

        let sub = MenuStyle {
            width: 161,
            ..MenuStyle::submenu(&theme)
        };
        let sl = MenuLayout::compute_styled(&sub, &items[..1], usize::MAX);
        assert_eq!(sl.rows[0], Rect::new(1, 1, 159, 20));
        assert_eq!(sl.height, 22);
        let parent = Rect::new(0, 783, l.width, l.height);
        let row = Rect::new(22, 785, 171, 32);
        assert_eq!(
            place_submenu_inset(
                parent,
                row,
                (sl.width, sl.height),
                Rect::new(0, 0, 1920, 1050),
                l.inset.right,
                sl.inset,
            ),
            (192, 784)
        );
    }

    fn lit_arrow_ink(theme: &Theme, style: MenuStyle, s: i32) -> u32 {
        let text = TextRenderer::load(&FontConfig::default());
        let items = vec![MenuItem::entry("").submenu(true)];
        let theme = Theme {
            scale: s,
            ..theme.clone()
        };
        let style = MenuStyle {
            width: 160,
            ..style
        };
        let l = MenuLayout::compute_styled(&style, &items, usize::MAX).scaled(s);
        let mut c = Canvas::new(l.width as u32, l.height as u32);
        draw_menu(&mut c, &theme, &text, &l, &items, Some(0), "", false);
        let r = l.rows[0];
        c.get(r.right() - ARROW_RIGHT_INSET * s, centred(&r, 7, s))
    }

    #[test]
    fn a_lit_start_menu_row_keeps_its_arrow_black_under_blue() {
        for v in crate::blue::Variant::ALL {
            let theme = crate::interfaces::blue_theme(v);
            let cs = &theme.colors;
            for s in [1, 2] {
                for style in [MenuStyle::root(&theme), MenuStyle::submenu(&theme)] {
                    assert_eq!(
                        lit_arrow_ink(&theme, style, s),
                        cs.menu_text.argb(),
                        "{v:?} s={s}: a Start menu's lit arrow"
                    );
                }
                let context = MenuStyle::context(&theme, ContextBitmaps::None);
                assert_eq!(
                    lit_arrow_ink(&theme, context, s),
                    cs.hilight_text.argb(),
                    "{v:?} s={s}: a context menu's lit arrow"
                );
            }
        }
        let classic = wide_theme();
        for style in [MenuStyle::root(&classic), MenuStyle::submenu(&classic)] {
            assert_eq!(
                lit_arrow_ink(&classic, style, 1),
                classic.colors.hilight_text.argb(),
                "Classic's Start menu"
            );
        }
    }

    #[test]
    fn a_flat_menu_doubles_at_scale_2() {
        let text = TextRenderer::load(&FontConfig::default());
        let items = chrome_rows();
        let draw = |c: &mut Canvas, p: &crate::look::Paint| {
            let theme = Theme::for_draw(p, 11 * p.s as u32);
            let style = MenuStyle::context(&Theme::default(), ContextBitmaps::None);
            let style = MenuStyle { width: 80, ..style };
            let l = MenuLayout::compute_styled(&style, &items, usize::MAX).scaled(p.s);
            draw_menu_at(
                c,
                (2 * p.s, 2 * p.s),
                &theme,
                &text,
                &l,
                &items,
                Some(0),
                "",
                false,
            );
        };
        assert!(crate::look::styled::fixture::blue_doubles_at_paint_scale_2(
            90, 110, draw
        ));
    }
}
