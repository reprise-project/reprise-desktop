//! The shell view: the file list of the Open/Save As dialog and of Explorer's
//! folder window, in all five view modes.
//!
//! - **List: row pitch 17, column pitch 196, fixed.** A label too long for the
//!   cell is ellipsized rather than widening the column, so `layout` feeds a
//!   fixed `iconview::Cell { w: 196, h: 17 }` through `iconview`'s column-major
//!   flow instead of measuring labels.
//! - **Icon 16 px** at the cell's left edge, the label rect starting at
//!   `cell.x + ICON_PX` with no gap, flush to the top of the 17 px row.
//! - **Horizontal scrollbar only**, the look's scrollbar thickness tall
//!   (`ControlMetrics::scroll`, 16 px in Classic). Icons are not clipped at the body edge.
//!
//! Details delegates to `listview` through a transient `ListView` over
//! `sv.entries` (`with_details`, `details_listview`). The icon modes
//! (Thumbnails, Tiles, Icons) live in [`crate::controls::shellgrid`]. Icons
//! renames through `iconview`'s box, the other modes through `listview`'s.

use crate::access::{self, Tree, accesskit};
use crate::classic::widgets::{
    self, Axis, BarHit, FIELD_PAD, ScrollHit, ScrollParts, axis_lit, bar_hit, scrollbar_parts,
    scrollbar_pos_at,
};
use crate::controls::iconview::{self, Cell, IconMove};
use crate::controls::listview;
use crate::controls::metrics::ControlMetrics;
use crate::controls::scrollbar;
use crate::controls::shellgrid;
use crate::look::Paint;
use crate::look::widgets::item::{self, Mark};
use crate::look::widgets::{draw_field_frame, draw_hscrollbar, draw_scrollbar};
use crate::pointer::{Lit, Pointer, WHEEL_ROWS};
use crate::raster::{Canvas, Rect};
use crate::selection::{Mods, Selection, scroll_into_view};
use crate::text::{Align, TextRenderer};
use crate::textedit::TextEdit;
use crate::theme::{Color, TextUnits};
use accesskit::{NodeId, Role};
use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Rc;

pub const ROW_H: i32 = 17;
pub const ICON_PX: i32 = 16;
const COL_W: i32 = 196;

/// List mode's cell, grown with the text by `u`.
fn list_cell(u: TextUnits) -> Cell {
    Cell {
        w: u.len(COL_W),
        h: u.len(ROW_H),
    }
}

pub use crate::controls::shellgrid::{
    GROUP_HEADER_H, ICON_CELL, THUMB_CELL, THUMB_FRAME, TILE_CELL, TILE_ICON,
};

/// One file or folder. The host fills the fields its modes need: `icon`
/// (16 px) for List and Details, `icon32` for Icons, `icon48` for Tiles,
/// `thumb` for Thumbnails. `id` is the host's own handle (a `filedlg::FolderId`
/// for a directory, whatever it keys its files by otherwise); reprisekit only carries it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    pub id: u64,
    pub name: String,
    pub icon: Option<std::rc::Rc<Canvas>>,
    pub icon32: Option<std::rc::Rc<Canvas>>,
    pub icon48: Option<std::rc::Rc<Canvas>>,
    pub thumb: Option<Canvas>,
    pub is_dir: bool,
    /// A shortcut: gets `iconview::shortcut_overlay` in the three icon modes. A
    /// shortcut to a folder lists as a file ([`Entry::is_folder`]).
    pub link: bool,
    pub cut: bool,
    pub size: u64,
    pub kind: String,
    pub modified: String,
    /// `modified` as seconds since the epoch, what [`sort`] orders Date Modified by.
    pub mtime: Option<i64>,
    pub detail: Vec<String>,
    /// The Original Location column's text, for a Recycle Bin item.
    pub folder: String,
    pub deleted: String,
}

impl Entry {
    /// Sorts among the folders and shows no size; a shortcut to a folder lists among the files with its own size.
    pub fn is_folder(&self) -> bool {
        self.is_dir && !self.link
    }
}

/// A group header, drawn before entry `first`. The host orders the entries: a
/// group covers `first` up to the next group's `first`; `groups` must be sorted and within `entries`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Group {
    pub title: String,
    pub first: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    List,
    Details,
    Thumbnails,
    Tiles,
    Icons,
}

/// The view's state. The host owns selection, scroll position and the entries; `shellview` reads them and reports hits.
#[derive(Clone, Debug, PartialEq)]
pub struct ShellView {
    /// The part of either scrollbar the pointer rests on or holds, tracked by the host.
    pub bars: Pointer<(Axis, ScrollHit)>,
    /// Details' header column the pointer rests on or holds, tracked by the host.
    pub header: Pointer<usize>,
    pub entries: Vec<Entry>,
    /// Group headers, in entry order. Empty for an ungrouped view; ignored by List and Details.
    pub groups: Vec<Group>,
    pub mode: Mode,
    pub selection: Selection,
    /// The column Details sorts by and whether ascending; it gets the header arrow.
    pub sort: Option<(usize, bool)>,
    /// Horizontal scroll offset in px: List's column scroll and Details' both.
    pub left: i32,
    /// Vertical scroll offset: **rows** in Details (drives the transient
    /// `ListView`'s `top`), **px** in the icon modes. Unused by List. A mode
    /// change goes through `set_mode`, which resets it.
    pub top: usize,
    pub focus_ring: bool,
    /// A marquee drag's rectangle in `Layout`'s coordinates (the caller's, not
    /// the body's), drawn over the cells as a dotted rectangle. The host owns the
    /// drag and feeds the same rect to `Selection::marquee`.
    pub marquee: Option<Rect>,
    pub edit: Option<(usize, TextEdit)>,
    /// The entry a drag is hovering: highlighted like a selected one, never focused.
    pub drop_target: Option<usize>,
    /// Details' columns, seeded from `details_columns`; a host persists its widths here across a resize.
    /// Widths are 100% px, as every list column's ([`resize_column`] maps a drag back).
    pub columns: Vec<listview::Column>,
    pub fields: Vec<Field>,
    /// The entry under the pointer ([`ShellView::hover`]), lit by a look that boxes its items.
    pub hot: Option<usize>,
    /// The view lacks the focus: a look that boxes its items greys the selection.
    pub inactive: bool,
}

impl Default for ShellView {
    fn default() -> Self {
        ShellView {
            entries: Vec::new(),
            groups: Vec::new(),
            mode: Mode::List,
            selection: Selection::default(),
            sort: None,
            left: 0,
            top: 0,
            focus_ring: false,
            marquee: None,
            edit: None,
            drop_target: None,
            columns: details_columns(),
            fields: details_fields(),
            bars: Pointer::new(),
            header: Pointer::new(),
            hot: None,
            inactive: false,
        }
    }
}

impl ShellView {
    /// The pointer moved to (`x`, `y`): the entry under it turns hot. True when
    /// that changed what a look that boxes its items draws.
    pub fn hover(&mut self, lay: &Layout, x: i32, y: i32) -> bool {
        let hot = match hit(self, lay, x, y) {
            Hit::Entry(i) => Some(i),
            _ => None,
        };
        std::mem::replace(&mut self.hot, hot) != hot
    }

    pub fn unhover(&mut self) -> bool {
        self.hot.take().is_some()
    }

    pub(crate) fn mark(&self, i: usize) -> Mark {
        let drop = self.drop_target == Some(i);
        Mark {
            selected: self.selection.is(i) || drop,
            hot: self.hot == Some(i) || drop,
            lead: self.selection.focus == Some(i),
            dim: self.inactive,
            ring: self.focus_ring && self.selection.focus == Some(i) && !drop,
        }
    }

    /// Switch to `mode`, scrolled to the top-left: `top` and `left` mean different units in each mode.
    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
        self.top = 0;
        self.left = 0;
    }
}

/// Where everything in the view sits, in the caller's unscaled coordinates.
/// `cells` is one rect per entry, already shifted by `left` / `top`; `labels`
/// is the label rect inside each (`label_rect`), `headers` one rect per group.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub frame: Rect,
    pub body: Rect,
    pub cells: Vec<Rect>,
    pub headers: Vec<Rect>,
    pub hscroll: Option<Rect>,
    pub vscroll: Option<Rect>,
    /// The content's full width and height, in the units `left` and `top` use for this mode.
    pub content: (i32, i32),
    /// The look's control sizes it was laid out with, which Details' `listview` layout is rebuilt at.
    pub controls: ControlMetrics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Entry(usize),
    Background,
    Header(usize),
    Divider(usize),
    VBar(BarHit),
    HBar(BarHit),
    Nothing,
}

/// Rows per column and the columns that takes for `sv.entries`, given an
/// `area` as tall as the body. The column count can't reuse
/// `iconview::columns`, which answers "how many fit", not "how many are needed".
fn rows_and_cols(sv: &ShellView, area: Rect, u: TextUnits) -> (usize, usize) {
    let rows = iconview::rows_per_column(list_cell(u), area);
    let cols = sv.entries.len().div_ceil(rows);
    (rows, cols)
}

/// Details' four columns in Explorer's fixed order: Name 178, Size 94 and
/// Type 118 wide, Size aligned right, Date Modified 120.
pub fn details_columns() -> Vec<listview::Column> {
    vec![
        listview::Column::new("Name", 178),
        listview::Column {
            title: "Size".into(),
            width: 94,
            right: true,
        },
        listview::Column::new("Type", 118),
        listview::Column::new("Date Modified", 120),
    ]
}

/// Details' column `col`, `from` 100% px wide when its divider was pressed,
/// dragged `dx` px at text size `u`: it takes the longest 100% width that grows
/// to at most the dragged edge, held to `listview::MIN_COL_W`. `true` when it changed.
pub fn resize_column(sv: &mut ShellView, col: usize, from: i32, dx: i32, u: TextUnits) -> bool {
    let Some(c) = sv.columns.get_mut(col) else {
        return false;
    };
    let width = u.within(u.len(from) + dx).max(listview::MIN_COL_W);
    std::mem::replace(&mut c.width, width) != width
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Name,
    Folder,
    Size,
    Kind,
    Modified,
    Deleted,
}

pub fn details_fields() -> Vec<Field> {
    vec![Field::Name, Field::Size, Field::Kind, Field::Modified]
}

/// The Recycle Bin's six columns: Name 178, Original Location 180, Date
/// Deleted 120, Size 96 (right-aligned), Type 120, Date Modified 120.
pub fn bin_columns() -> Vec<listview::Column> {
    vec![
        listview::Column::new("Name", 178),
        listview::Column::new("Original Location", 180),
        listview::Column::new("Date Deleted", 120),
        listview::Column {
            title: "Size".into(),
            width: 96,
            right: true,
        },
        listview::Column::new("Type", 120),
        listview::Column::new("Date Modified", 120),
    ]
}

pub fn bin_fields() -> Vec<Field> {
    vec![
        Field::Name,
        Field::Folder,
        Field::Deleted,
        Field::Size,
        Field::Kind,
        Field::Modified,
    ]
}

/// Tiles' lines under `e`'s name: its kind and [`size_text`]; none for a folder.
/// A picture's [`image_detail`] replaces them once its size is known.
pub fn tile_detail(e: &Entry) -> Vec<String> {
    if e.is_folder() {
        return Vec::new();
    }
    vec![e.kind.clone(), size_text(e.size)]
}

/// Tiles' lines under a picture's name: its `dims` over its `kind`.
pub fn image_detail(dims: (u32, u32), kind: &str) -> Vec<String> {
    vec![format!("{} x {}", dims.0, dims.1), kind.to_string()]
}

/// The square inside [`THUMB_FRAME`]'s 1 px border that a thumbnail fits in.
pub const THUMB_INNER: u32 = (THUMB_FRAME - 2) as u32;

/// `thumb` of a `dims` picture as an [`Entry::thumb`]: fitted inside [`THUMB_INNER`], never past
/// the picture's own size, in device pixels at `scale`.
pub fn frame_thumb(thumb: Canvas, dims: (u32, u32), scale: i32) -> Canvas {
    let s = scale.max(1) as u32;
    let (lw, lh) = crate::raster::fit_size(dims.0, dims.1, (THUMB_INNER, THUMB_INNER));
    let (w, h) = (lw.max(1) * s, lh.max(1) * s);
    let (sw, sh) = thumb.device_size();
    let device = if (sw, sh) == (w, h) {
        thumb
    } else {
        Canvas::resample_box_from(sw, sh, w, h, |x, y| thumb.pixels[(y * sw + x) as usize])
    };
    device.at_scale(scale)
}

/// A file's size in Explorer's Size column and under a tile's name: bytes
/// rounded up to whole KB, grouped in threes (0 reads "0 KB", 1 and 1024
/// "1 KB", 1025 "2 KB", 12,345,678 "12,057 KB"). The status bar, Details
/// pane and infotip give three figures instead.
pub fn size_text(bytes: u64) -> String {
    let kb = bytes.div_ceil(1024).to_string();
    let mut out = String::with_capacity(kb.len() + kb.len() / 3 + 3);
    for (i, d) in kb.chars().enumerate() {
        if i > 0 && (kb.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(d);
    }
    out.push_str(" KB");
    out
}

/// The same column again flips its direction, another sorts ascending. Then [`sort`].
pub fn sort_by_column(sv: &mut ShellView, col: usize) {
    let asc = match sv.sort {
        Some((c, asc)) if c == col => !asc,
        _ => true,
    };
    sv.sort = Some((col, asc));
    sort(sv);
}

/// Puts `sv.entries` in `sv.sort`'s order, the selection following its rows.
/// Ascending lists folders ([`Entry::is_folder`]) first, each part by the
/// column's key (name and type case-insensitively, size and date numerically),
/// then by name; descending is that order reversed, so files come first. A
/// view with no sort, or with groups, keeps the host's order.
pub fn sort(sv: &mut ShellView) {
    let Some((col, asc)) = sv.sort else {
        return;
    };
    if !sv.groups.is_empty() {
        return;
    }
    let field = sv.fields.get(col).copied().unwrap_or(Field::Name);
    #[derive(PartialEq, Eq, PartialOrd, Ord)]
    enum Key {
        Name,
        Size(u64),
        Text(String),
        Date(Option<i64>),
    }
    let key = |e: &Entry| {
        let by = match field {
            Field::Name => Key::Name,
            Field::Size => Key::Size(e.size),
            Field::Kind => Key::Text(e.kind.to_lowercase()),
            Field::Folder => Key::Text(e.folder.to_lowercase()),
            Field::Deleted => Key::Text(e.deleted.clone()),
            Field::Modified => Key::Date(e.mtime),
        };
        (!e.is_folder(), by, e.name.to_lowercase(), e.name.clone())
    };
    let mut order: Vec<usize> = (0..sv.entries.len()).collect();
    order.sort_by_cached_key(|&i| key(&sv.entries[i]));
    if !asc {
        order.reverse();
    }
    let mut entries: Vec<Option<Entry>> = std::mem::take(&mut sv.entries)
        .into_iter()
        .map(Some)
        .collect();
    sv.entries = order
        .iter()
        .map(|&i| entries[i].take().expect("each index once"))
        .collect();
    sv.selection.reorder(&order);
}

/// `sv.entries` as Details rows through `sv.fields`: a folder's size cell is
/// empty, a file's is `size_text`.
pub struct DetailsRows<'a>(pub &'a [Entry], pub &'a [Field]);

impl listview::Rows for DetailsRows<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }
    fn icon(&self, row: usize) -> Option<&Canvas> {
        self.0.get(row).and_then(|e| e.icon.as_deref())
    }
    fn cell(&self, row: usize, col: usize) -> Cow<'_, str> {
        let Some(e) = self.0.get(row) else {
            return Cow::Borrowed("");
        };
        match self.1.get(col) {
            Some(Field::Name) => Cow::Borrowed(&e.name),
            Some(Field::Folder) => Cow::Borrowed(&e.folder),
            Some(Field::Size) if e.is_folder() => Cow::Borrowed(""),
            Some(Field::Size) => Cow::Owned(size_text(e.size)),
            Some(Field::Kind) => Cow::Borrowed(&e.kind),
            Some(Field::Modified) => Cow::Borrowed(&e.modified),
            Some(Field::Deleted) => Cow::Borrowed(&e.deleted),
            None => Cow::Borrowed(""),
        }
    }
}

/// The transient `listview::ListView` a Details arm builds over `sv`; it
/// borrows the columns and selection, so a layout, hit or draw copies nothing large.
type DetailsView<'a> = listview::ListView<DetailsRows<'a>, &'a [listview::Column], &'a Selection>;

fn details_listview(sv: &ShellView) -> DetailsView<'_> {
    listview::ListView {
        columns: &sv.columns,
        rows: DetailsRows(&sv.entries, &sv.fields),
        selection: &sv.selection,
        top: sv.top,
        left: sv.left,
        sort: sv.sort,
        sort_tint: true,
        focus_ring: sv.focus_ring,
        edit: sv.edit.clone(),
        drop_row: sv.drop_target,
        checks: None,
        held_check: Default::default(),
        groups: Vec::new(),
        frame: true,
        bars: sv.bars,
        header: sv.header,
        hot: sv.hot,
        inactive: sv.inactive,
        clicks: Default::default(),
    }
}

/// Run `f` on a Details `ListView` that owns `sv`'s columns, selection and
/// rename for the call (moved out, not copied), then move changes back into `sv`.
fn with_details<T>(
    sv: &mut ShellView,
    f: impl FnOnce(&mut listview::ListView<DetailsRows<'_>>) -> T,
) -> T {
    let ShellView {
        entries,
        selection,
        sort,
        left,
        top,
        focus_ring,
        edit,
        drop_target,
        columns,
        fields,
        bars,
        header,
        ..
    } = sv;
    let mut lv = listview::ListView {
        columns: std::mem::take(columns),
        rows: DetailsRows(entries, fields),
        selection: std::mem::take(selection),
        top: *top,
        left: *left,
        sort: *sort,
        sort_tint: true,
        focus_ring: *focus_ring,
        edit: edit.take(),
        drop_row: *drop_target,
        checks: None,
        held_check: Default::default(),
        groups: Vec::new(),
        frame: true,
        bars: *bars,
        header: *header,
        hot: None,
        inactive: false,
        clicks: Default::default(),
    };
    let out = f(&mut lv);
    *columns = lv.columns;
    *selection = lv.selection;
    *top = lv.top;
    *left = lv.left;
    *focus_ring = lv.focus_ring;
    *edit = lv.edit;
    out
}

/// Entry `i`'s label rect, for the rename box and click tests against the
/// label rather than the cell. Each mode's is what `draw` fills with the
/// highlight: the name's ink padded as that mode pads it. Measured on demand:
/// measuring every label per layout made a pointer move O(folder).
pub fn label_rect(sv: &ShellView, lay: &Layout, i: usize, text: &TextRenderer, px: u32) -> Rect {
    if matches!(sv.mode, Mode::Details) {
        let lv = details_listview(sv);
        let lv_lay = listview::layout(&lv, lay.frame, &lay.controls);
        return lv.edit_rect(&lv_lay, i, text, px);
    }
    let (Some(&cell), Some(e)) = (lay.cells.get(i), sv.entries.get(i)) else {
        return Rect::new(0, 0, 0, 0);
    };
    match sv.mode {
        Mode::List => {
            let w = (cell.width - ICON_PX).max(0);
            let shown = text.ellipsize(&e.name, w, false, px);
            Rect::new(
                cell.x + ICON_PX,
                cell.y,
                text.measure(&shown, false, px).min(w),
                cell.height,
            )
        }
        mode => shellgrid::label_of(mode, cell, e, text, px, lay.controls.text),
    }
}

/// `sv`'s layout in `r`. Measures nothing: every mode's cells are a fixed
/// pitch at the text size, and label rects come from `label_rect` on demand.
/// List: a sunken field (`FIELD_PAD` inset, matching the frame `draw` puts
/// round it) holding the fixed-pitch grid, with a horizontal scrollbar along
/// the bottom once the columns overflow, never a vertical one. The three icon
/// modes flow `grid` in the same field and take a vertical scrollbar once the
/// content overruns it, never a horizontal one. Details delegates to
/// `listview::layout` and reports its rows as `cells`; `body` spans the header
/// and rows together, so a cell's `y` sits the header height below `body.y`.
pub fn layout(sv: &ShellView, r: Rect, cm: &ControlMetrics) -> Layout {
    let frame = r;
    match sv.mode {
        Mode::List => {
            let sunken = frame.inset(FIELD_PAD);
            let cell = list_cell(cm.text);
            let (_, cols) = rows_and_cols(sv, sunken, cm.text);
            let overflows = cols as i32 * cell.w > sunken.width;
            let (body, hscroll) = if overflows {
                let h = (sunken.height - cm.scroll).max(0);
                let bar = Rect::new(sunken.x, sunken.y + h, sunken.width, cm.scroll);
                (Rect::new(sunken.x, sunken.y, sunken.width, h), Some(bar))
            } else {
                (sunken, None)
            };
            // The bar's height comes off the rows, which can push entries into another column: the range is the columns `body` holds.
            let content_w = rows_and_cols(sv, body, cm.text).1 as i32 * cell.w;
            let left = scrollbar::held(sv.left, body.width, content_w);
            let cells: Vec<Rect> = iconview::layout(cell, body, sv.entries.len())
                .into_iter()
                .map(|c| Rect::new(c.x - left, c.y, c.width, c.height))
                .collect();
            Layout {
                frame,
                body,
                cells,
                headers: Vec::new(),
                hscroll,
                vscroll: None,
                content: (content_w, body.height),
                controls: *cm,
            }
        }
        Mode::Details => {
            let lv = details_listview(sv);
            let lv_lay = listview::layout(&lv, frame, cm);
            // One rect per entry: a row scrolled out of view keeps its place above or below the body.
            let row_h = lv_lay.row_h();
            let cells: Vec<Rect> = (0..sv.entries.len())
                .map(|i| {
                    Rect::new(
                        lv_lay.body.x,
                        lv_lay.body.y + (i as i64 - sv.top as i64) as i32 * row_h,
                        lv_lay.body.width,
                        row_h,
                    )
                })
                .collect();
            let body = Rect::new(
                lv_lay.header.x,
                lv_lay.header.y,
                lv_lay.header.width,
                lv_lay.header.height + lv_lay.body.height,
            );
            Layout {
                frame,
                body,
                cells,
                headers: Vec::new(),
                hscroll: lv_lay.hscroll,
                vscroll: lv_lay.scrollbar,
                content: (lv_lay.total_w, sv.entries.len() as i32 * row_h),
                controls: *cm,
            }
        }
        Mode::Thumbnails | Mode::Tiles | Mode::Icons => {
            let sunken = frame.inset(FIELD_PAD);
            let grid = shellgrid::Grid::of(sv.mode, cm.text);
            let cell = grid.cell;
            // Reserving the bar narrows the body, which can need one more row: two
            // rounds reach the fixed point, as `listview::layout` does.
            let mut vbar = false;
            let mut body = sunken;
            let mut content_h = 0;
            for _ in 0..2 {
                body = Rect::new(
                    sunken.x,
                    sunken.y,
                    sunken.width - if vbar { cm.scroll } else { 0 },
                    sunken.height,
                );
                let h = shellgrid::grid_height(&grid, body, sv.entries.len(), &sv.groups);
                content_h = h;
                vbar |= h > body.height;
            }
            let (cells, headers) =
                shellgrid::grid(&grid, body, sv.entries.len(), &sv.groups, sv.top as i32);
            Layout {
                frame,
                body,
                cells,
                headers,
                hscroll: None,
                vscroll: vbar.then(|| Rect::new(body.right(), body.y, cm.scroll, body.height)),
                content: ((body.width / cell.0).max(1) * cell.0, content_h),
                controls: *cm,
            }
        }
    }
}

/// The last `layout` a host asked for, handed out again while nothing it
/// reads has changed, so layout on every pointer motion and paint builds the cells once per change.
#[derive(Debug, Default)]
pub struct LayoutCache(RefCell<Option<(LayoutKey, Rc<Layout>)>>);

/// Everything `layout` reads: rect, the look's sizes, mode, entry count, both scroll offsets, groups and Details columns.
#[derive(Debug)]
struct LayoutKey {
    r: Rect,
    cm: ControlMetrics,
    mode: Mode,
    len: usize,
    top: usize,
    left: i32,
    groups: Vec<Group>,
    columns: Vec<listview::Column>,
}

impl LayoutKey {
    fn of(sv: &ShellView, r: Rect, cm: &ControlMetrics) -> Self {
        LayoutKey {
            r,
            cm: *cm,
            mode: sv.mode,
            len: sv.entries.len(),
            top: sv.top,
            left: sv.left,
            groups: sv.groups.clone(),
            columns: sv.columns.clone(),
        }
    }

    fn matches(&self, sv: &ShellView, r: Rect, cm: &ControlMetrics) -> bool {
        self.r == r
            && self.cm == *cm
            && self.mode == sv.mode
            && self.len == sv.entries.len()
            && self.top == sv.top
            && self.left == sv.left
            && self.groups == sv.groups
            && self.columns == sv.columns
    }
}

impl LayoutCache {
    pub fn get(&self, sv: &ShellView, r: Rect, cm: &ControlMetrics) -> Rc<Layout> {
        let mut slot = self.0.borrow_mut();
        if let Some((key, lay)) = slot.as_ref()
            && key.matches(sv, r, cm)
        {
            return Rc::clone(lay);
        }
        let lay = Rc::new(layout(sv, r, cm));
        *slot = Some((LayoutKey::of(sv, r, cm), Rc::clone(&lay)));
        lay
    }
}

/// What's under `(x, y)`. Details delegates to the transient `ListView`'s
/// `hit`, translating `Hit::Row` to `Hit::Entry`. The icon modes test the
/// vertical scrollbar, then the cells, and report a group header as `Background`.
pub fn hit(sv: &ShellView, lay: &Layout, x: i32, y: i32) -> Hit {
    if matches!(sv.mode, Mode::Details) {
        let lv = details_listview(sv);
        let lv_lay = listview::layout(&lv, lay.frame, &lay.controls);
        return match lv.hit(&lv_lay, x, y) {
            listview::Hit::Row(i) => Hit::Entry(i),
            listview::Hit::Header(i) => Hit::Header(i),
            listview::Hit::Divider(i) => Hit::Divider(i),
            listview::Hit::Background => Hit::Background,
            listview::Hit::VBar(b) => Hit::VBar(b),
            listview::Hit::HBar(b) => Hit::HBar(b),
            listview::Hit::Check(_) | listview::Hit::Nothing => Hit::Nothing,
        };
    }
    if let Some(vb) = lay.vscroll.filter(|vb| vb.contains(x, y)) {
        return Hit::VBar(bar_hit(&icon_vparts(sv, lay, vb), x, y, false));
    }
    if let Some(hb) = lay.hscroll
        && hb.contains(x, y)
    {
        return Hit::HBar(bar_hit(&list_hparts(sv, lay, hb), x, y, true));
    }
    if lay.body.contains(x, y) {
        return lay
            .cells
            .iter()
            .position(|c| c.contains(x, y))
            .map_or(Hit::Background, Hit::Entry);
    }
    Hit::Nothing
}

/// The entry pointed at by its icon or name, what an infotip hangs from: any
/// point of the cell in the icon modes, only the icon and label in List and
/// Details, where the space after a short name and other columns point at nothing.
pub fn name_hit(
    sv: &ShellView,
    lay: &Layout,
    x: i32,
    y: i32,
    text: &TextRenderer,
    px: u32,
) -> Option<usize> {
    let Hit::Entry(i) = hit(sv, lay, x, y) else {
        return None;
    };
    let named = match sv.mode {
        // The name as its column cuts it short, not the rename box's rect, which runs on for the whole name.
        Mode::Details => {
            let lv = details_listview(sv);
            let lv_lay = listview::layout(&lv, lay.frame, &lay.controls);
            x < lv.name_rect(&lv_lay, i, text, px).right()
        }
        Mode::List => x < label_rect(sv, lay, i, text, px).right(),
        _ => true,
    };
    named.then_some(i)
}

/// Keep the focused cell inside `lay.body`, sliding by the minimum distance
/// (`listview::ListView::follow`'s rule) on each mode's axis. `step` calls this
/// on every move; `type_ahead` has no `Layout`, so a host calls `follow` after it.
pub fn follow(sv: &mut ShellView, lay: &Layout) {
    if let Some(i) = sv.selection.focus {
        ensure_visible(sv, i, lay);
    }
}

/// Scroll cell `i` into view: `sv.left` in List, `sv.top` in the icon modes
/// (px) and Details (rows). Does nothing when `i` is already inside `lay.body`.
pub fn ensure_visible(sv: &mut ShellView, i: usize, lay: &Layout) {
    let Some(&cell) = lay.cells.get(i) else {
        return;
    };
    match sv.mode {
        Mode::List => {
            let left = list_left(sv, lay);
            if cell.x < lay.body.x {
                sv.left = (left - (lay.body.x - cell.x)).max(0);
            } else if cell.right() > lay.body.right() {
                sv.left = left + cell.right() - lay.body.right();
            }
        }
        Mode::Details => {
            let row_h = lay.controls.text.len(listview::ROW_H);
            let rows = (lay.body.height - lay.controls.list_header).max(row_h) / row_h;
            scroll_into_view(&mut sv.top, i, rows as usize);
        }
        _ => {
            if cell.y < lay.body.y {
                sv.top = sv.top.saturating_sub((lay.body.y - cell.y) as usize);
            } else if cell.bottom() > lay.body.bottom() {
                sv.top += (cell.bottom() - lay.body.bottom()) as usize;
            }
        }
    }
}

/// Scroll `by` steps: rows in Details, a cell's height per step in the icon
/// modes, and in List a column sideways per wheel notch (`WHEEL_ROWS` steps),
/// as the bar's arrows step. `true` when the offset moved.
pub fn scroll(sv: &mut ShellView, by: i32, lay: &Layout) -> bool {
    match sv.mode {
        Mode::List => {
            let notches = by.signum() * (by.abs() / WHEEL_ROWS).max(1);
            hscroll(sv, notches * list_cell(lay.controls.text).w, lay)
        }
        Mode::Details => with_details(sv, |lv| {
            let lv_lay = listview::layout(lv, lay.frame, &lay.controls);
            lv.scroll(by as i64, &lv_lay)
        }),
        _ => {
            let pitch = shellgrid::Grid::of(sv.mode, lay.controls.text).cell.1;
            let max = (lay.content.1 - lay.body.height).max(0) as i64;
            let top = (sv.top as i64 + by as i64 * pitch as i64).clamp(0, max) as usize;
            let changed = top != sv.top;
            sv.top = top;
            changed
        }
    }
}

/// Scroll by a page of rows, as a click on the vertical track does; `true` when the offset moved.
pub fn page(sv: &mut ShellView, by: i32, lay: &Layout) -> bool {
    match sv.mode {
        Mode::Details => {
            let lv = details_listview(sv);
            let rows = listview::layout(&lv, lay.frame, &lay.controls).rows.max(1) as i32;
            scroll(sv, by * rows, lay)
        }
        Mode::List => false,
        _ => {
            let pitch = shellgrid::Grid::of(sv.mode, lay.controls.text)
                .cell
                .1
                .max(1);
            scroll(sv, by * (lay.body.height / pitch).max(1), lay)
        }
    }
}

/// List's horizontal bar parts: `left` and the columns' width in px, as `draw` hands the bar.
fn list_hparts(sv: &ShellView, lay: &Layout, hb: Rect) -> ScrollParts {
    scrollbar_parts(
        hb,
        1,
        list_left(sv, lay) as usize,
        lay.body.width.max(0) as usize,
        lay.content.0.max(0) as usize,
        true,
        lay.controls.thumb_min,
    )
}

/// How far List's columns are scrolled under `lay`: `sv.left` held to what
/// overflows the body. A window widening after a scroll leaves `sv.left` past
/// the end; this keeps the last column's end at the body's.
fn list_left(sv: &ShellView, lay: &Layout) -> i32 {
    scrollbar::held(sv.left, lay.body.width, lay.content.0)
}

/// Scroll sideways by `by` px, clamped to what overflows: List's columns, or
/// Details' through its `listview`. The icon modes never scroll sideways.
pub fn hscroll(sv: &mut ShellView, by: i32, lay: &Layout) -> bool {
    match sv.mode {
        Mode::List => {
            let from = list_left(sv, lay);
            sv.left = scrollbar::held(from + by, lay.body.width, lay.content.0);
            sv.left != from
        }
        Mode::Details => with_details(sv, |lv| {
            let lv_lay = listview::layout(lv, lay.frame, &lay.controls);
            lv.hscroll(by, &lv_lay)
        }),
        _ => false,
    }
}

/// How far one arrow press scrolls Details sideways, in every look.
const DETAILS_NUDGE: i32 = 16;

pub fn hstep(sv: &mut ShellView, dir: i32, lay: &Layout) -> bool {
    let step = match sv.mode {
        Mode::List => list_cell(lay.controls.text).w,
        _ => DETAILS_NUDGE,
    };
    hscroll(sv, dir.signum() * step, lay)
}

pub fn hpage(sv: &mut ShellView, by: i32, lay: &Layout) -> bool {
    hscroll(sv, by * lay.body.width, lay)
}

/// The horizontal thumb, grabbed `grab` px right of its left edge, dragged to pointer `x`.
pub fn drag_hthumb(sv: &mut ShellView, x: i32, grab: i32, lay: &Layout) -> bool {
    match sv.mode {
        Mode::List => {
            let Some(hb) = lay.hscroll else {
                return false;
            };
            let p = list_hparts(sv, lay, hb);
            let visible = lay.body.width.max(0) as usize;
            let total = lay.content.0.max(0) as usize;
            let left = scrollbar_pos_at(&p, x - grab, visible, total, true) as i32;
            let changed = left != sv.left;
            sv.left = left;
            changed
        }
        Mode::Details => with_details(sv, |lv| {
            let lv_lay = listview::layout(lv, lay.frame, &lay.controls);
            let left = lv.left_at(&lv_lay, x, grab);
            let changed = left != lv.left;
            lv.left = left;
            changed
        }),
        _ => false,
    }
}

/// The icon modes' vertical bar parts: `top` and content height are px, as `draw` hands the bar.
fn icon_vparts(sv: &ShellView, lay: &Layout, vb: Rect) -> ScrollParts {
    scrollbar_parts(
        vb,
        1,
        sv.top,
        lay.body.height.max(0) as usize,
        lay.content.1.max(0) as usize,
        false,
        lay.controls.thumb_min,
    )
}

/// The vertical thumb, grabbed `grab` px below its top, dragged to pointer
/// `y`. In Details the thumb is `listview`'s; List never scrolls vertically.
pub fn drag_thumb(sv: &mut ShellView, y: i32, grab: i32, lay: &Layout) -> bool {
    match sv.mode {
        Mode::List => false,
        Mode::Details => with_details(sv, |lv| {
            let lv_lay = listview::layout(lv, lay.frame, &lay.controls);
            let top = lv.top_at(&lv_lay, y, grab);
            let changed = top != lv.top;
            lv.top = top;
            changed
        }),
        _ => {
            let Some(vb) = lay.vscroll else {
                return false;
            };
            let p = icon_vparts(sv, lay, vb);
            let visible = lay.body.height.max(0) as usize;
            let top = scrollbar_pos_at(&p, y - grab, visible, lay.content.1.max(0) as usize, false);
            let changed = top != sv.top;
            sv.top = top;
            changed
        }
    }
}

/// Move the focus by `dir` (`iconview::move_selection`'s rule: the nearest
/// cell in a 90-degree cone), let `mods` decide the selection, then scroll it
/// into view; `true` when the focus moved. Details uses `details_step`.
pub fn step(sv: &mut ShellView, dir: IconMove, lay: &Layout, mods: Mods) -> bool {
    if matches!(sv.mode, Mode::Details) {
        return details_step(sv, dir, lay, mods);
    }
    let next = iconview::move_selection(sv.selection.focus, &lay.cells, dir);
    if next == sv.selection.focus {
        return false;
    }
    let Some(to) = next else { return false };
    sv.selection.move_focus(to, mods);
    sv.focus_ring = true;
    ensure_visible(sv, to, lay);
    true
}

/// Details' arm of `step`: Up/Down move a row, Home/End jump to the ends, Left/Right do nothing.
fn details_step(sv: &mut ShellView, dir: IconMove, lay: &Layout, mods: Mods) -> bool {
    let row_step = match dir {
        IconMove::Up => -1,
        IconMove::Down => 1,
        IconMove::Home => i64::MIN / 2,
        IconMove::End => i64::MAX / 2,
        IconMove::Left | IconMove::Right => return false,
    };
    with_details(sv, |lv| {
        let lv_lay = listview::layout(lv, lay.frame, &lay.controls);
        lv.step(row_step, &lv_lay, mods)
    })
}

/// Jump to the next entry whose name starts with `ch`, case-insensitively,
/// searching from just after the focus and wrapping; `true` when something matched.
pub fn type_ahead(sv: &mut ShellView, ch: char) -> bool {
    let names = sv.entries.iter().map(|e| e.name.as_str());
    match crate::selection::type_ahead(names, sv.selection.focus, ch) {
        Some(i) => {
            sv.selection.set_only(Some(i));
            true
        }
        None => false,
    }
}

/// The rename box over entry `i`'s label, for every mode but Icons (which has
/// `iconview`'s wrapped box) and Details (where the transient `ListView` places
/// its own): `listview`'s single-line box, padded as it pads a name cell.
pub(crate) fn draw_rename(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    sv: &ShellView,
    i: usize,
    label: Rect,
    px: u32,
) {
    let Paint { s, .. } = *p;
    let Some((_, ed)) = sv.edit.as_ref().filter(|(j, _)| *j == i) else {
        return;
    };
    let w = (text.measure(&ed.text, false, px) + listview::EDIT_PAD_L + listview::EDIT_PAD_R) * s;
    let box_r = Rect::new(
        label.x - listview::EDIT_BACK * s,
        label.y - s,
        w,
        p.text.len(listview::EDIT_H) * s,
    );
    listview::draw_rename_box(c, text, p, ed, box_r, px);
}

impl Hit {
    pub fn bar(self) -> Option<(Axis, ScrollHit)> {
        match self {
            Hit::VBar(b) => Some((Axis::Vertical, b.part()?)),
            Hit::HBar(b) => Some((Axis::Horizontal, b.part()?)),
            _ => None,
        }
    }
}

pub fn draw(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    sv: &ShellView,
    lay: &Layout,
    bars: Lit<(Axis, ScrollHit)>,
    px: u32,
) {
    let Paint { cs, s, .. } = *p;
    if matches!(sv.mode, Mode::Details) {
        let lv = details_listview(sv);
        let lv_lay = listview::layout(&lv, lay.frame, &lay.controls);
        listview::draw(c, text, p, &lv, &lv_lay, bars, px);
        return;
    }
    // A list: the style's list line (as Details' list view draws), not its edit line.
    let frame = lay.frame.scaled(s);
    c.fill_rect(frame, cs.window);
    draw_field_frame(c, p, frame, cs.window);
    if !matches!(sv.mode, Mode::List) {
        shellgrid::draw_grid(c, text, p, sv, lay, px);
        if let Some(vb) = lay.vscroll {
            draw_scrollbar(
                c,
                p,
                vb.scaled(s),
                sv.top,
                lay.body.height.max(0) as usize,
                lay.content.1.max(0) as usize,
                axis_lit(bars, Axis::Vertical),
            );
        }
        return;
    }
    for (i, (cell, entry)) in lay.cells.iter().zip(&sv.entries).enumerate() {
        let visible = cell.intersect(&lay.body);
        if visible.is_empty() {
            continue;
        }
        let selected = sv.selection.is(i) || sv.drop_target == Some(i);
        let cell_s = cell.scaled(s);
        let visible_s = visible.scaled(s);
        let label_full = Rect::new(
            cell_s.x + ICON_PX * s,
            cell_s.y,
            (cell_s.width - ICON_PX * s).max(0),
            cell_s.height,
        );
        let editing = matches!(&sv.edit, Some((j, _)) if *j == i);
        let shown = text.ellipsize(&entry.name, label_full.width, false, px);
        // A look that boxes its items boxes the icon and name, 3 px past its ink, under both; the text stays.
        let ink_w = text.measure(&shown, false, px);
        let item_box = Rect::new(
            cell_s.x,
            cell_s.y,
            ICON_PX * s + ink_w + 3 * s,
            cell_s.height,
        )
        .intersect(&visible_s);
        let mark = if editing { Mark::default() } else { sv.mark(i) };
        let boxed = item::draw_item(c, p, item_box, mark);
        if let Some(icon) = shellgrid::entry_icon(entry, Mode::List) {
            let dy = (cell.height - ICON_PX) / 2 * s;
            c.blit_over(cell_s.x, cell_s.y + dy, &icon);
        }
        let label = label_full.intersect(&visible_s);
        if label.is_empty() {
            continue;
        }
        if editing {
            let label = label_rect(sv, lay, i, text, px);
            draw_rename(c, text, p, sv, i, label.scaled(s), px);
            continue;
        }
        let fg = if selected && !boxed {
            c.fill_rect(label, cs.hilight);
            cs.hilight_text
        } else {
            cs.window_text
        };
        text.draw(c, label, &shown, false, fg, Align::Left, px);
    }
    if let Some(hb) = lay.hscroll {
        draw_hscrollbar(
            c,
            p,
            hb.scaled(s),
            list_left(sv, lay) as usize,
            lay.body.width.max(0) as usize,
            lay.content.0.max(0) as usize,
            axis_lit(bars, Axis::Horizontal),
        );
    }
    if let Some(m) = sv.marquee {
        let m = m.intersect(&lay.body).scaled(s);
        if !m.is_empty() {
            crate::look::widgets::draw_marquee(c, p, m, cs.window_text);
        }
    }
}

/// A six-entry view in `mode`, `grouped` splitting it into "Folders" and
/// "Files": one entry selected and focused, one cut, one a shortcut, and flat
/// coloured squares for icons, so each mode shows where its icon and label go.
/// Used by `examples/gallery.rs` and the preview test.
pub fn sample(mode: Mode, grouped: bool) -> ShellView {
    let square = |n: u32, c: Color| {
        let mut sq = Canvas::new(n, n);
        sq.fill_rect(Rect::new(0, 0, n as i32, n as i32), c);
        sq
    };
    let folder = Color::rgb(255, 214, 112);
    let file = Color::rgb(198, 208, 224);
    let rows = [
        ("My Music", true, "File Folder", ""),
        ("My Pictures", true, "File Folder", ""),
        ("hills.jpg", false, "JPEG Image", "800 x 600"),
        ("notes.txt", false, "Text Document", "1 KB"),
        ("report.odt", false, "OpenDocument Text", "28 KB"),
        ("setup.exe", false, "Application", "412 KB"),
    ];
    let entries: Vec<Entry> = rows
        .iter()
        .enumerate()
        .map(|(i, (name, is_dir, kind, detail))| {
            let tint = if *is_dir { folder } else { file };
            Entry {
                id: i as u64,
                name: (*name).into(),
                icon: Some(square(16, tint).into()),
                icon32: Some(std::rc::Rc::new(square(32, tint))),
                icon48: Some(std::rc::Rc::new(square(48, tint))),
                thumb: Some(square(80, tint)),
                is_dir: *is_dir,
                link: i == 5,
                cut: i == 4,
                size: 1024 * (i as u64 + 1),
                kind: (*kind).into(),
                modified: "14/09/2026 21:40".into(),
                mtime: None,
                detail: if detail.is_empty() {
                    vec![(*kind).into()]
                } else {
                    vec![(*detail).into(), (*kind).into()]
                },
                folder: String::new(),
                deleted: String::new(),
            }
        })
        .collect();
    let mut sv = ShellView {
        entries,
        groups: if grouped {
            vec![
                Group {
                    title: "Folders".into(),
                    first: 0,
                },
                Group {
                    title: "Files".into(),
                    first: 2,
                },
            ]
        } else {
            Vec::new()
        },
        mode,
        sort: Some((0, true)),
        focus_ring: true,
        ..Default::default()
    };
    sv.selection.set_only(Some(2));
    sv
}

/// The shell view's `access` part, the same in every mode: a list with one
/// option per entry, named by its name and described by its Details columns
/// ("Size: 2 KB, Type: Text Document"), the selected ones marked. Entries
/// scrolled out of the body have no bounds. A rename in progress is a text
/// input. Returns the node that has the focus: the rename field, else the focused entry, else the list.
pub fn access(
    sv: &ShellView,
    lay: &Layout,
    text: &TextRenderer,
    px: u32,
    tree: &mut Tree,
    parent: NodeId,
    key: impl std::hash::Hash,
) -> NodeId {
    let mut n = access::node(Role::ListBox, lay.frame);
    n.set_multiselectable();
    access::focusable(&mut n, true);
    let id = tree.push(parent, key, n);
    let rows = DetailsRows(&sv.entries, &sv.fields);
    let mut focus = id;
    for (i, e) in sv.entries.iter().enumerate() {
        let mut o = accesskit::Node::new(Role::ListBoxOption);
        o.set_label(e.name.as_str());
        let rest: Vec<String> = sv
            .columns
            .iter()
            .enumerate()
            .skip(1)
            .filter_map(|(k, c)| {
                let v = listview::Rows::cell(&rows, i, k);
                (!v.is_empty()).then(|| format!("{}: {v}", c.title))
            })
            .collect();
        if !rest.is_empty() {
            o.set_description(rest.join(", "));
        }
        o.set_selected(sv.selection.is(i));
        if let Some(cell) = lay.cells.get(i)
            && !cell.intersect(&lay.body).is_empty()
        {
            o.set_bounds(access::bounds(*cell));
        }
        let oid = tree.push(id, e.id, o);
        if sv.selection.focus == Some(i) {
            focus = oid;
        }
        if let Some((k, edit)) = &sv.edit
            && *k == i
        {
            let r = label_rect(sv, lay, i, text, px);
            focus = widgets::access_edit(edit, 0, false, r, true, text, px, tree, oid, "rename");
        }
    }
    focus
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::look::classic_paint;
    use crate::textedit::TextEdit;
    use crate::theme::{ColorScheme, FontConfig, Theme};

    #[test]
    fn a_thumbnail_fits_the_frame_at_the_windows_scale() {
        let big = Canvas::from_pixels(256, 192, vec![0xff00_00ff; 256 * 192]);
        let one = frame_thumb(big.clone(), (800, 600), 1);
        assert_eq!((one.width, one.height, one.scale()), (94, 71, 1));
        assert_eq!(one.get(10, 10), 0xff00_00ff);
        let two = frame_thumb(big, (800, 600), 2);
        assert_eq!((two.width, two.height, two.scale()), (94, 71, 2));
        assert_eq!(two.device_size(), (188, 142));
        let small = Canvas::from_pixels(94, 71, vec![0xff00_00ff; 94 * 71]);
        assert_eq!(frame_thumb(small, (800, 600), 2).device_size(), (188, 142));
        let tiny = Canvas::from_pixels(16, 8, vec![1; 128]);
        let t = frame_thumb(tiny.clone(), (16, 8), 1);
        assert_eq!(t, tiny);
        assert_eq!(frame_thumb(tiny, (16, 8), 3).device_size(), (48, 24));
    }

    pub(crate) fn rend() -> (TextRenderer, u32) {
        (
            TextRenderer::load(&FontConfig::default()),
            Theme::default().fonts.size_px,
        )
    }

    pub(crate) fn entry(name: &str) -> Entry {
        Entry {
            name: name.into(),
            kind: "File".into(),
            ..Default::default()
        }
    }

    pub(crate) fn view(n: usize) -> ShellView {
        ShellView {
            entries: (0..n).map(|i| entry(&format!("item{i}"))).collect(),
            ..Default::default()
        }
    }

    pub(crate) fn square(n: u32, color: Color) -> Canvas {
        let mut c = Canvas::new(n, n);
        c.fill_rect(Rect::new(0, 0, n as i32, n as i32), color);
        c
    }

    /// The cache hands back the same layout while nothing `layout` reads changes, and a fresh one equal to `layout`'s after any input does.
    #[test]
    fn layout_cache_rebuilds_only_when_an_input_changes() {
        let cache = LayoutCache::default();
        let mut sv = view(40);
        sv.mode = Mode::Icons;
        let r = Rect::new(0, 0, 300, 200);
        let first = cache.get(&sv, r, &ControlMetrics::CLASSIC);
        sv.selection.set_only(Some(3));
        sv.marquee = Some(Rect::new(1, 1, 5, 5));
        assert!(
            Rc::ptr_eq(&first, &cache.get(&sv, r, &ControlMetrics::CLASSIC)),
            "a selection or marquee change reuses the cached layout"
        );
        let changes: [fn(&mut ShellView, &mut Rect); 7] = [
            |_, r| r.width += 1,
            |sv, _| sv.set_mode(Mode::Details),
            |sv, _| sv.entries.truncate(10),
            |sv, _| sv.top = 1,
            |sv, _| sv.left = 5,
            |sv, _| sv.columns[0].width += 10,
            |sv, _| {
                sv.groups = vec![Group {
                    title: "A".into(),
                    first: 0,
                }]
            },
        ];
        let mut r = r;
        for (n, change) in changes.iter().enumerate() {
            let before = cache.get(&sv, r, &ControlMetrics::CLASSIC);
            change(&mut sv, &mut r);
            let after = cache.get(&sv, r, &ControlMetrics::CLASSIC);
            assert!(
                !Rc::ptr_eq(&before, &after),
                "change {n} rebuilds the layout"
            );
            assert_eq!(
                *after,
                layout(&sv, r, &ControlMetrics::CLASSIC),
                "change {n} caches layout's answer"
            );
        }
        let thick = ControlMetrics {
            scroll: 17,
            ..ControlMetrics::CLASSIC
        };
        let before = cache.get(&sv, r, &ControlMetrics::CLASSIC);
        let after = cache.get(&sv, r, &thick);
        assert!(!Rc::ptr_eq(&before, &after), "a look's sizes rebuild it");
        assert_eq!(*after, layout(&sv, r, &thick));
    }

    /// A List layout built before `entries` shrank draws the remaining entries and skips the stale cells.
    #[test]
    fn list_draw_skips_cells_past_the_entries() {
        let (text, px) = rend();
        let mut sv = view(6);
        let r = Rect::new(0, 0, 400, 200);
        let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
        sv.entries.truncate(2);
        let mut c = Canvas::new(400, 200);
        draw(
            &mut c,
            &text,
            &classic_paint(&ColorScheme::default(), 1),
            &sv,
            &lay,
            None,
            px,
        );
    }

    /// List flows down a column then into the next, at the 17 px row / 196 px column pitch.
    #[test]
    fn list_mode_flows_column_major() {
        const EXPECTED_ROW_H: i32 = 17;
        const EXPECTED_COL_W: i32 = 196;
        let sv = view(40);
        // A body exactly 3 rows tall (17 * 3 plus the field's 3 px inset each side), so a fourth entry wraps to x + 196.
        let r = Rect::new(0, 0, 3000, 3 * EXPECTED_ROW_H + 2 * FIELD_PAD);
        let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
        assert_eq!(lay.cells[0].y, lay.cells[1].y - EXPECTED_ROW_H);
        assert_eq!(
            lay.cells[3].y, lay.cells[0].y,
            "the fourth wraps to a new column"
        );
        assert_eq!(lay.cells[3].x, lay.cells[0].x + EXPECTED_COL_W);
    }

    /// More entries than fit gets a horizontal scrollbar; a view that holds them all gets none. List never scrolls vertically.
    #[test]
    fn list_mode_scrolls_horizontally_only_when_it_overflows() {
        let narrow = layout(
            &view(2),
            Rect::new(0, 0, 400, 200),
            &ControlMetrics::CLASSIC,
        );
        assert!(narrow.hscroll.is_none(), "two entries fit");
        assert!(narrow.vscroll.is_none());
        let full = layout(
            &view(400),
            Rect::new(0, 0, 400, 200),
            &ControlMetrics::CLASSIC,
        );
        assert!(full.hscroll.is_some());
        assert!(full.vscroll.is_none(), "List mode never scrolls vertically");
    }

    /// Right steps into the next column, Down within one, and neither walks off the end.
    #[test]
    fn arrows_walk_the_columns() {
        let mut sv = view(6);
        let r = Rect::new(0, 0, 400, 3 * ROW_H + 2 * FIELD_PAD);
        let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
        sv.selection.set_only(Some(0));
        assert!(step(&mut sv, IconMove::Down, &lay, Mods::default()));
        assert_eq!(sv.selection.focus, Some(1));
        assert!(step(&mut sv, IconMove::Right, &lay, Mods::default()));
        assert_eq!(sv.selection.focus, Some(4));
        sv.selection.set_only(Some(5));
        assert!(!step(&mut sv, IconMove::Right, &lay, Mods::default()));
        assert_eq!(sv.selection.focus, Some(5), "the last column stays put");
    }

    /// Typing a letter jumps to the next entry starting with it, wrapping round.
    #[test]
    fn type_ahead_jumps_to_the_next_match() {
        let mut sv = ShellView {
            entries: ["alpha", "beta", "bravo", "delta", "École"]
                .iter()
                .map(|n| entry(n))
                .collect(),
            ..view(0)
        };
        sv.selection.set_only(Some(0));
        assert!(type_ahead(&mut sv, 'é'));
        assert_eq!(sv.selection.focus, Some(4));
        sv.selection.set_only(Some(0));
        assert!(type_ahead(&mut sv, 'b'));
        assert_eq!(sv.selection.focus, Some(1));
        assert!(type_ahead(&mut sv, 'b'));
        assert_eq!(
            sv.selection.focus,
            Some(2),
            "a second press goes to the next one"
        );
        assert!(!type_ahead(&mut sv, 'z'));
        assert_eq!(
            sv.selection.focus,
            Some(2),
            "no match leaves the selection alone"
        );
    }

    /// The selected entry's label is filled with the hilight colour and drawn in hilight text; nothing else is.
    #[test]
    fn the_selected_entry_is_filled_with_the_hilight() {
        const EXPECTED_ICON_PX: i32 = 16;
        let theme = Theme::default();
        let cs = &theme.colors;
        let (text, px) = rend();
        let mut sv = view(4);
        sv.selection.set_only(Some(1));
        let r = Rect::new(0, 0, 400, 100);
        let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
        let mut c = Canvas::new(400, 100);
        draw(&mut c, &text, &classic_paint(cs, 1), &sv, &lay, None, px);
        let sel = lay.cells[1];
        assert_eq!(
            c.get(sel.x + EXPECTED_ICON_PX + 2, sel.y + 2),
            cs.hilight.argb()
        );
        let other = lay.cells[0];
        assert_eq!(
            c.get(other.x + EXPECTED_ICON_PX + 2, other.y + 2),
            cs.window.argb()
        );
    }

    /// Stepping to a cell beyond the visible columns scrolls it into view.
    /// `shellview::step` must follow in px, since List scrolls by column pitch, not row count.
    #[test]
    fn stepping_past_the_visible_columns_scrolls_to_follow() {
        let mut sv = view(6);
        // One column wide, three rows tall once the scrollbar takes its thickness: column 1 (rows 3..6) is off screen.
        let r = Rect::new(
            0,
            0,
            COL_W + 2 * FIELD_PAD,
            3 * ROW_H + ControlMetrics::CLASSIC.scroll + 2 * FIELD_PAD,
        );
        let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
        sv.selection.set_only(Some(2)); // column 0's last row, on screen
        assert!(step(&mut sv, IconMove::Right, &lay, Mods::default()));
        assert_eq!(sv.selection.focus, Some(5), "column 1's last row");
        assert!(
            sv.left > 0,
            "must scroll to bring the new selection into view"
        );

        // Re-laying out at the new scroll position must put the selected cell
        // inside the body: draw skips a cell with an empty intersection.
        let lay2 = layout(&sv, r, &ControlMetrics::CLASSIC);
        let cell = lay2.cells[5];
        assert!(cell.x >= lay2.body.x, "not scrolled off the left");
        assert!(
            cell.right() <= lay2.body.right(),
            "not scrolled off the right"
        );
    }

    /// A label too long for its 196 px column is ellipsized, not given a wider
    /// column. Checks pixel-for-pixel that `draw` renders what `text::ellipsize`
    /// computes for the 180 px label width (196 column - 16 icon).
    #[test]
    fn an_overrunning_label_is_ellipsized_to_the_fixed_column() {
        const EXPECTED_LABEL_W: i32 = 180; // 196 px column - 16 px icon
        let theme = Theme::default();
        let cs = &theme.colors;
        let (text, px) = rend();
        let mut sv = view(1);
        let long = "Software Distribution and Setup Archive";
        sv.entries[0].name = long.into();
        assert!(
            text.measure(long, false, px) > EXPECTED_LABEL_W,
            "the name must not already fit, or this test proves nothing"
        );
        let shown = text.ellipsize(long, EXPECTED_LABEL_W, false, px);
        assert!(shown.len() < long.len(), "the shown text is trimmed");
        assert!(shown.ends_with("..."), "an end ellipsis, not a bare cut");

        let r = Rect::new(0, 0, 400, 100);
        let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
        let cell = lay.cells[0];

        let mut got = Canvas::new(400, 100);
        draw(&mut got, &text, &classic_paint(cs, 1), &sv, &lay, None, px);

        let mut want = Canvas::new(400, 100);
        want.fill_rect(Rect::new(0, 0, 400, 100), cs.window);
        let label = Rect::new(cell.x + 16, cell.y, EXPECTED_LABEL_W, cell.height);
        text.draw(
            &mut want,
            label,
            &shown,
            false,
            cs.window_text,
            Align::Left,
            px,
        );

        for y in cell.y..cell.bottom() {
            for x in cell.x..cell.right() {
                assert_eq!(
                    got.get(x, y),
                    want.get(x, y),
                    "pixel ({x},{y}) differs: draw must show exactly the ellipsized text"
                );
            }
        }
    }

    /// A file's size rounds up to whole KB; a folder shows none.
    #[test]
    fn size_text_rounds_up_to_kb() {
        assert_eq!(size_text(0), "0 KB");
        assert_eq!(size_text(1), "1 KB");
        assert_eq!(size_text(1024), "1 KB");
        assert_eq!(size_text(1025), "2 KB");
        assert_eq!(size_text(6_500), "7 KB");
        assert_eq!(size_text(150_000), "147 KB");
        assert_eq!(size_text(1_024_000), "1,000 KB");
        assert_eq!(size_text(1_234_567), "1,206 KB");
        assert_eq!(size_text(12_345_678), "12,057 KB");
        assert_eq!(size_text(1 << 40), "1,073,741,824 KB");
    }

    #[test]
    fn a_tile_names_a_folder_alone_and_a_file_with_its_kind_and_size() {
        let e = |is_dir, link, kind: &str, size| Entry {
            is_dir,
            link,
            kind: kind.into(),
            size,
            ..Default::default()
        };
        assert!(tile_detail(&e(true, false, "File Folder", 4096)).is_empty());
        let tile = |size| tile_detail(&e(false, false, "Text Document", size));
        assert_eq!(tile(2_048), ["Text Document", "2 KB"]);
        assert_eq!(tile(6_500)[1], "7 KB");
        assert_eq!(
            tile_detail(&e(true, true, "Shortcut", 4096)),
            ["Shortcut", "4 KB"]
        );
    }

    #[test]
    fn tiles_writes_a_pictures_dimensions_over_its_kind() {
        assert_eq!(
            image_detail((800, 600), "JPEG Image"),
            ["800 x 600", "JPEG Image"]
        );
    }

    /// Details lays out over listview's grid: a row is its height and the first starts below the header.
    #[test]
    fn details_rows_sit_under_the_header() {
        const EXPECTED_HEADER_H: i32 = 17;
        const EXPECTED_ROW_H: i32 = 17;
        let mut sv = view(4);
        sv.mode = Mode::Details;
        let lay = layout(&sv, Rect::new(0, 0, 400, 200), &ControlMetrics::CLASSIC);
        assert_eq!(lay.cells[0].height, EXPECTED_ROW_H);
        assert_eq!(lay.cells[0].y, lay.body.y + EXPECTED_HEADER_H);
        assert_eq!(lay.cells[1].y, lay.cells[0].y + EXPECTED_ROW_H);
    }

    /// A folder shows no size and its type is "File Folder"; a shortcut to a folder shows its own size.
    #[test]
    fn a_folder_has_no_size_column() {
        let mut sv = view(0);
        sv.mode = Mode::Details;
        sv.entries = vec![
            Entry {
                name: "Docs".into(),
                is_dir: true,
                kind: "File Folder".into(),
                modified: "1/1/2003 9:00 AM".into(),
                ..Default::default()
            },
            Entry {
                name: "Docs link".into(),
                is_dir: true,
                link: true,
                size: 4,
                kind: "Shortcut".into(),
                ..Default::default()
            },
        ];
        use listview::Rows;
        let rows = DetailsRows(&sv.entries, &sv.fields);
        assert_eq!(rows.cell(0, 1), "", "a folder shows no size");
        assert_eq!(rows.cell(0, 2), "File Folder");
        assert_eq!(rows.cell(1, 1), "1 KB", "a shortcut to a folder is sized");
    }

    /// The Recycle Bin's columns: Name 178, Original Location 180, Date Deleted
    /// 120, Size 96 right-aligned, Type 120, Date Modified 120; location and date from the entry's `folder` and `deleted`.
    #[test]
    fn bin_columns_show_where_and_when_an_item_was_deleted() {
        let cols = bin_columns();
        let shape: Vec<(&str, i32, bool)> = cols
            .iter()
            .map(|c| (c.title.as_str(), c.width, c.right))
            .collect();
        assert_eq!(
            shape,
            [
                ("Name", 178, false),
                ("Original Location", 180, false),
                ("Date Deleted", 120, false),
                ("Size", 96, true),
                ("Type", 120, false),
                ("Date Modified", 120, false),
            ]
        );
        let entries = vec![Entry {
            name: "notes.txt".into(),
            folder: "/home/u/Desktop".into(),
            deleted: "9/14/2026 3:43 PM".into(),
            size: 4096,
            ..Default::default()
        }];
        use listview::Rows;
        let fields = bin_fields();
        assert_eq!(fields.len(), cols.len());
        let rows = DetailsRows(&entries, &fields);
        assert_eq!(rows.cell(0, 1), "/home/u/Desktop");
        assert_eq!(rows.cell(0, 2), "9/14/2026 3:43 PM");
        assert_eq!(rows.cell(0, 3), "4 KB");
    }

    /// Clicking a row reports its index and a column header reports that column,
    /// straight off the transient `ListView`; stepping Down moves the selection as `listview::ListView::step` would.
    #[test]
    fn details_hit_and_step_delegate_to_listview() {
        let mut sv = view(3);
        sv.mode = Mode::Details;
        let r = Rect::new(0, 0, 400, 200);
        let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
        assert_eq!(
            hit(&sv, &lay, lay.cells[1].x + 5, lay.cells[1].y + 5),
            Hit::Entry(1)
        );
        assert_eq!(
            hit(&sv, &lay, 5, lay.body.y + 2),
            Hit::Header(0),
            "the header, not a row"
        );
        sv.selection.set_only(Some(0));
        assert!(step(&mut sv, IconMove::Down, &lay, Mods::default()));
        assert_eq!(sv.selection.focus, Some(1));
        assert!(
            !step(&mut sv, IconMove::Left, &lay, Mods::default()),
            "a single-column Details view has no row to Left's left"
        );
    }

    pub(crate) fn render(sv: &ShellView, r: Rect) -> (Canvas, Layout, Theme) {
        let (text, px) = rend();
        let theme = Theme::default();
        let lay = layout(sv, r, &ControlMetrics::CLASSIC);
        let mut c = Canvas::new(r.width as u32, r.height as u32);
        draw(
            &mut c,
            &text,
            &classic_paint(&theme.colors, 1),
            sv,
            &lay,
            None,
            px,
        );
        (c, lay, theme)
    }

    pub(crate) fn iconed(n: usize, mode: Mode) -> ShellView {
        let mut sv = view(n);
        sv.mode = mode;
        for e in &mut sv.entries {
            e.icon32 = Some(std::rc::Rc::new(square(32, Color::rgb(255, 0, 0))));
            e.icon48 = Some(std::rc::Rc::new(square(48, Color::rgb(255, 0, 0))));
        }
        sv
    }

    /// `hit` names the cell under the pointer, the background between them, a group header as background and the scrollbar's three parts.
    #[test]
    fn hit_maps_cells_headers_background_and_the_scrollbar() {
        let mut sv = iconed(60, Mode::Icons);
        sv.groups = vec![Group {
            title: "Folders".into(),
            first: 0,
        }];
        let r = Rect::new(0, 0, 400, 200);
        let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
        let cell = lay.cells[1];
        assert_eq!(hit(&sv, &lay, cell.x + 2, cell.y + 2), Hit::Entry(1));
        let header = lay.headers[0];
        assert_eq!(
            hit(&sv, &lay, header.x + 2, header.y + 2),
            Hit::Background,
            "a group header is not an entry"
        );
        let vb = lay.vscroll.expect("60 icons overrun a 200 px body");
        assert_eq!(hit(&sv, &lay, vb.x + 2, vb.y + 2), Hit::VBar(BarHit::Back));
        assert_eq!(
            hit(&sv, &lay, vb.x + 2, vb.bottom() - 2),
            Hit::VBar(BarHit::Forward)
        );
        // At the top the thumb sits under the up arrow: the track's middle is a
        // page down, the thumb reports its grab, and a drag to the track's end scrolls to the last page.
        assert_eq!(
            hit(&sv, &lay, vb.x + 2, vb.y + vb.height / 2),
            Hit::VBar(BarHit::PageForward)
        );
        assert_eq!(
            hit(
                &sv,
                &lay,
                vb.x + 2,
                vb.y + ControlMetrics::CLASSIC.scroll + 2
            ),
            Hit::VBar(BarHit::Thumb(2))
        );
        assert!(drag_thumb(&mut sv, vb.bottom(), 2, &lay));
        assert_eq!(sv.top, (lay.content.1 - lay.body.height) as usize);
        assert_eq!(
            hit(
                &sv,
                &lay,
                vb.x + 2,
                vb.y + ControlMetrics::CLASSIC.scroll + 2
            ),
            Hit::VBar(BarHit::PageBack),
            "scrolled down, the track's top is a page up"
        );
        assert!(!drag_thumb(&mut sv, vb.bottom(), 2, &lay), "already there");
        assert_eq!(hit(&sv, &lay, 0, 0), Hit::Nothing, "the frame itself");
    }

    /// List's range is the columns its body holds once the bar has taken its
    /// height off the rows: scrolled to the far end, the last entry's column ends
    /// at the body's edge. A wider window shows its own far end, and an arrow steps from there.
    #[test]
    fn list_mode_scrolls_to_its_last_column_and_no_further() {
        let mut sv = view(60);
        sv.mode = Mode::List;
        let cm = ControlMetrics::CLASSIC;
        let r = Rect::new(0, 0, 300, 100);
        let lay = layout(&sv, r, &cm);
        let hb = lay.hscroll.expect("60 names overrun 300 px");
        drag_hthumb(&mut sv, hb.right(), 2, &lay);
        let lay = layout(&sv, r, &cm);
        assert_eq!(
            lay.cells[59].right(),
            lay.body.right(),
            "the last column ends there"
        );
        assert!(!hstep(&mut sv, 1, &lay), "and no further");

        let far = sv.left;
        let wide = layout(&sv, Rect::new(0, 0, 1000, 100), &cm);
        assert!(wide.hscroll.is_some());
        let end = wide.content.0 - wide.body.width;
        assert!(end < far);
        assert_eq!(wide.cells[59].right(), wide.body.right());
        assert!(!hstep(&mut sv, 1, &wide), "already at the far end");
        assert!(hstep(&mut sv, -1, &wide));
        assert_eq!(sv.left, end - COL_W);
    }

    /// In List the wheel scrolls sideways a column a notch, clamped.
    #[test]
    fn the_wheel_steps_list_mode_a_column_a_notch() {
        let mut sv = view(60);
        sv.mode = Mode::List;
        let lay = layout(&sv, Rect::new(0, 0, 300, 100), &ControlMetrics::CLASSIC);
        assert!(!scroll(&mut sv, -WHEEL_ROWS, &lay), "already at the left");
        assert!(scroll(&mut sv, WHEEL_ROWS, &lay));
        assert_eq!(sv.left, COL_W);
        assert!(scroll(&mut sv, 2 * WHEEL_ROWS, &lay));
        assert_eq!(sv.left, 3 * COL_W);
        assert!(scroll(&mut sv, -1, &lay), "a part notch still steps");
        assert_eq!(sv.left, 2 * COL_W);
        assert_eq!(sv.top, 0, "nothing scrolls down");
    }

    /// List's bar: the arrows step a column, the track pages, the thumb reports
    /// its grab and a drag to the track's end lands on the last page; Details' bar does the same through its `listview`.
    #[test]
    fn the_horizontal_bar_steps_pages_and_drags() {
        let mut sv = view(60);
        sv.mode = Mode::List;
        let r = Rect::new(0, 0, 300, 100);
        let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
        let hb = lay.hscroll.expect("60 names overrun 300 px");
        let max = lay.content.0 - lay.body.width;
        assert!(max > 0);
        assert_eq!(hit(&sv, &lay, hb.x + 2, hb.y + 2), Hit::HBar(BarHit::Back));
        assert_eq!(
            hit(&sv, &lay, hb.right() - 2, hb.y + 2),
            Hit::HBar(BarHit::Forward)
        );
        assert_eq!(
            hit(
                &sv,
                &lay,
                hb.x + ControlMetrics::CLASSIC.scroll + 2,
                hb.y + 2
            ),
            Hit::HBar(BarHit::Thumb(2))
        );
        assert_eq!(
            hit(
                &sv,
                &lay,
                hb.right() - ControlMetrics::CLASSIC.scroll - 2,
                hb.y + 2
            ),
            Hit::HBar(BarHit::PageForward)
        );
        assert!(hstep(&mut sv, 1, &lay));
        assert_eq!(sv.left, COL_W);
        assert!(hpage(&mut sv, 1, &lay));
        assert_eq!(sv.left, COL_W + lay.body.width);
        assert!(drag_hthumb(&mut sv, hb.right(), 2, &lay));
        assert_eq!(sv.left, max, "the last page");
        assert!(!hstep(&mut sv, 1, &lay), "clamped");
        assert_eq!(
            hit(
                &sv,
                &lay,
                hb.x + ControlMetrics::CLASSIC.scroll + 2,
                hb.y + 2
            ),
            Hit::HBar(BarHit::PageBack)
        );
        assert!(hpage(&mut sv, -99, &lay));
        assert_eq!(sv.left, 0);

        sv.mode = Mode::Details;
        sv.columns[0].width = 400;
        let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
        let hb = lay.hscroll.expect("a 400 px column overruns 300 px");
        assert_eq!(
            hit(
                &sv,
                &lay,
                hb.x + ControlMetrics::CLASSIC.scroll + 2,
                hb.y + 2
            ),
            Hit::HBar(BarHit::Thumb(2))
        );
        assert!(drag_hthumb(&mut sv, hb.right(), 2, &lay));
        assert!(sv.left > 0, "Details scrolled through its listview");
        assert!(hpage(&mut sv, -99, &lay));
        assert_eq!(sv.left, 0);
    }

    /// Details' widths are 100% px: at 200% every column, its divider and the
    /// scrolled width are twice as wide, and at 100% they draw as saved.
    #[test]
    fn details_columns_draw_grown_by_the_text_size() {
        let mut sv = view(3);
        sv.mode = Mode::Details;
        let r = Rect::new(0, 0, 400, 200);
        let saved: i32 = sv.columns.iter().map(|c| c.width).sum();
        for (pct, k) in [(100, 1), (200, 2)] {
            let cm = crate::theme::Theme::default()
                .with_text_size(pct)
                .controls();
            let lay = layout(&sv, r, &cm);
            assert_eq!(lay.content.0, k * saved, "{pct}%: the columns' width");
            let edge = FIELD_PAD + k * 178;
            assert_eq!(
                hit(&sv, &lay, edge, FIELD_PAD + 2),
                Hit::Divider(0),
                "{pct}%"
            );
        }
        assert_eq!(sv.columns[0].width, 178, "laying out keeps the saved width");
    }

    /// A divider dragged 40 px at 200% widens its column by 20 100% px; the same drag at 100% by 40.
    #[test]
    fn a_divider_drag_saves_its_width_at_100_percent() {
        let mut sv = view(3);
        let big = crate::theme::Theme::default()
            .with_text_size(200)
            .controls()
            .text;
        assert!(resize_column(&mut sv, 2, 118, 40, big));
        assert_eq!(sv.columns[2].width, 138);
        assert!(
            !resize_column(&mut sv, 2, 118, 41, big),
            "the edge stays at or left of the pointer"
        );
        assert!(resize_column(&mut sv, 2, 138, 40, TextUnits::ONE));
        assert_eq!(sv.columns[2].width, 178);
        assert!(resize_column(&mut sv, 2, 178, -400, big));
        assert_eq!(sv.columns[2].width, listview::MIN_COL_W);
        assert!(!resize_column(&mut sv, 9, 100, 40, big), "no such column");
    }

    /// Details re-lays its list out at the sizes its layout was built with: at
    /// 17 px the up arrow's last row, 16 px down the bar, is still the arrow, not the thumb.
    #[test]
    fn details_hits_a_17_px_bar_at_17_px() {
        let mut sv = view(40);
        sv.mode = Mode::Details;
        let cm = ControlMetrics {
            scroll: 17,
            ..ControlMetrics::CLASSIC
        };
        let lay = layout(&sv, Rect::new(0, 0, 300, 100), &cm);
        let vb = lay.vscroll.expect("40 rows overrun the body");
        assert_eq!(vb.width, 17);
        assert_eq!(hit(&sv, &lay, vb.x + 2, vb.y + 16), Hit::VBar(BarHit::Back));
        assert_eq!(
            hit(&sv, &lay, vb.x + 2, vb.y + 17),
            Hit::VBar(BarHit::Thumb(0))
        );
    }

    /// The List rename box is one row tall at every scale: 34 px at scale 2, its bottom border on the box's last row.
    #[test]
    fn list_rename_box_is_a_row_tall_at_scale_2() {
        let (text, px) = rend();
        let theme = Theme::default();
        let cs = &theme.colors;
        let mut sv = view(3);
        sv.edit = Some((0, TextEdit::new("item0", true)));
        let r = Rect::new(0, 0, 300, 100);
        let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
        let mut c = Canvas::new(600, 200);
        draw(&mut c, &text, &classic_paint(cs, 2), &sv, &lay, None, px);
        let label = label_rect(&sv, &lay, 0, &text, px).scaled(2);
        let (bx, by) = (label.x - listview::EDIT_BACK * 2, label.y - 2);
        assert_eq!(
            c.get(bx, by + 33),
            cs.window_frame.argb(),
            "border at row 33"
        );
        assert_ne!(c.get(bx, by + 34), cs.window_frame.argb(), "34 px tall");
    }

    /// Renaming in Details is the transient `ListView`'s own box, placed by `listview::ListView::edit_rect`.
    #[test]
    fn renaming_in_details_delegates_to_listview() {
        let mut sv = view(3);
        sv.mode = Mode::Details;
        sv.edit = Some((1, TextEdit::new("item1", true)));
        let (c, lay, theme) = render(&sv, Rect::new(0, 0, 400, 200));
        let cs = &theme.colors;
        let (text, px) = rend();
        let b = label_rect(&sv, &lay, 1, &text, px);
        assert_eq!(
            c.get(b.x, b.y + 2),
            cs.window_frame.argb(),
            "the rename box's left border"
        );
        assert_eq!(c.get(b.x + 1, b.y + 2), cs.window.argb(), "white inside");
    }

    /// An entry is named by its icon and label in List and Details, by its whole cell in the icon modes.
    #[test]
    fn a_name_hit_needs_the_icon_or_the_label_in_list_and_details() {
        let (text, px) = rend();
        let r = Rect::new(0, 0, 500, 200);
        for mode in [Mode::List, Mode::Details] {
            let mut sv = view(3);
            sv.mode = mode;
            let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
            let label = label_rect(&sv, &lay, 1, &text, px);
            let y = label.y + label.height / 2;
            // Past the name but still on the entry: the end of a List cell, the Size column of a Details row.
            let past = match mode {
                Mode::List => lay.cells[1].right() - 1,
                _ => label.right() + 80,
            };
            assert!(past >= label.right());
            assert_eq!(hit(&sv, &lay, past, y), Hit::Entry(1), "{mode:?}");
            assert_eq!(name_hit(&sv, &lay, label.x + 2, y, &text, px), Some(1));
            assert_eq!(name_hit(&sv, &lay, label.x - 8, y, &text, px), Some(1));
            assert_eq!(
                name_hit(&sv, &lay, past, y, &text, px),
                None,
                "{mode:?}: past the name"
            );
        }
        // A long name cut short in a narrow Name column names nothing past the column.
        let mut sv = view(3);
        sv.mode = Mode::Details;
        sv.entries[1].name = "a name far longer than its column".into();
        sv.columns[0].width = 80;
        let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
        let label = label_rect(&sv, &lay, 1, &text, px);
        let y = label.y + label.height / 2;
        let size = lay.body.x + 80 + 10;
        assert_eq!(hit(&sv, &lay, size, y), Hit::Entry(1));
        assert_eq!(
            name_hit(&sv, &lay, size, y, &text, px),
            None,
            "the Size cell"
        );
        assert_eq!(name_hit(&sv, &lay, label.x + 2, y, &text, px), Some(1));

        let mut sv = view(3);
        sv.mode = Mode::Icons;
        let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
        let cell = lay.cells[1];
        assert_eq!(
            name_hit(&sv, &lay, cell.x + 1, cell.y + 1, &text, px),
            Some(1)
        );
    }

    /// `scroll` moves by a cell's height in the icon modes and whole rows in
    /// Details; `ensure_visible` brings a cell below the body back into it.
    #[test]
    fn scrolling_moves_by_a_cell_and_ensure_visible_follows() {
        let mut sv = iconed(60, Mode::Icons);
        let r = Rect::new(0, 0, 400, 200);
        let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
        assert!(scroll(&mut sv, 1, &lay));
        assert_eq!(sv.top, ICON_CELL.1 as usize, "one cell down");
        sv.top = 0;
        let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
        let last = sv.entries.len() - 1;
        assert!(
            lay.cells[last].bottom() > lay.body.bottom(),
            "the last cell starts off screen"
        );
        ensure_visible(&mut sv, last, &lay);
        let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
        assert!(lay.cells[last].bottom() <= lay.body.bottom());
        assert!(lay.cells[last].y >= lay.body.y);
    }

    /// Writes target/shellview-preview.png: the five modes stacked, the last grouped, for a visual check.
    #[test]
    #[ignore]
    fn render_shellview() {
        let (text, px) = rend();
        let theme = Theme::default();
        let (w, h) = (700, 240);
        let views = [
            sample(Mode::List, false),
            sample(Mode::Details, false),
            sample(Mode::Icons, false),
            sample(Mode::Tiles, false),
            sample(Mode::Thumbnails, false),
            sample(Mode::Tiles, true),
        ];
        let mut c = Canvas::new(w as u32, (h * views.len() as i32) as u32);
        c.fill_rect(
            Rect::new(0, 0, w, h * views.len() as i32),
            theme.colors.button_face,
        );
        for (i, sv) in views.iter().enumerate() {
            let r = Rect::new(8, 8 + i as i32 * h, w - 16, h - 16);
            let lay = layout(sv, r, &ControlMetrics::CLASSIC);
            draw(
                &mut c,
                &text,
                &classic_paint(&theme.colors, 1),
                sv,
                &lay,
                None,
                px,
            );
        }
        crate::raster::write_preview(&c, "shellview-preview.png");
    }

    /// Details hands the lit bar part to the list it draws through: a Blue Details view lights its hot arrow, and `Hit::bar` names it.
    #[test]
    fn a_blue_details_view_lights_its_bar_part() {
        use crate::classic::widgets::{Axis, ScrollHit};
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let cm = theme.look.controls();
        let (text, px) = rend();
        let mut sv = view(80);
        sv.mode = Mode::Details;
        let r = Rect::new(0, 0, 300, 160);
        let lay = layout(&sv, r, &cm);
        let lv = details_listview(&sv);
        let sb = listview::layout(&lv, r, &cm)
            .scrollbar
            .expect("80 rows overflow");
        let up = hit(&sv, &lay, sb.x + 8, sb.y + 2).bar();
        assert_eq!(up, Some((Axis::Vertical, ScrollHit::Back)));
        sv.bars.motion(up);
        let mut c = Canvas::new(300, 160);
        draw(&mut c, &text, &theme.paint(), &sv, &lay, sv.bars.lit(), px);
        assert_eq!(
            c.get(sb.x + 8, sb.y + 2),
            crate::look::styled::fixture::blue_pixel(
                crate::style::Part::ScrollArrowUp,
                crate::style::State::Hot,
                8,
                2
            ),
            "the hot up arrow"
        );
    }

    /// Silver's icon and List modes take its list line, as its Details view does, not its edit line.
    #[test]
    fn silvers_view_takes_its_list_line_in_every_mode() {
        let theme = crate::interfaces::blue_theme(crate::blue::Variant::Silver);
        let cm = theme.look.controls();
        let silver = crate::blue::Variant::Silver.scheme();
        let list = silver.list_border.as_ref().unwrap().normal.rim.argb();
        let (text, px) = rend();
        let r = Rect::new(0, 0, 300, 160);
        for mode in [Mode::Icons, Mode::Tiles, Mode::List] {
            let mut sv = view(3);
            sv.mode = mode;
            let lay = layout(&sv, r, &cm);
            let mut c = Canvas::new(300, 160);
            draw(&mut c, &text, &theme.paint(), &sv, &lay, None, px);
            let f = lay.frame;
            for (x, y) in [(f.x + 40, f.y), (f.x, f.y + 40), (f.right() - 1, f.y + 40)] {
                assert_eq!(c.get(x, y), list, "{mode:?} ({x}, {y})");
            }
        }
    }

    /// Type sorts case-insensitively and Date Modified by time, not text;
    /// ascending keeps folders first, descending reverses it all. A view without a sort keeps the host's order.
    #[test]
    fn sort_orders_by_type_and_by_date() {
        let e = |name: &str, is_dir: bool, kind: &str, mtime: i64| Entry {
            name: name.into(),
            is_dir,
            kind: kind.into(),
            modified: format!("{mtime}"),
            mtime: Some(mtime),
            ..Default::default()
        };
        let mut sv = ShellView {
            entries: vec![
                e("notes", false, "text Document", 30),
                e("pics", true, "Folder", 10),
                e("a.jpg", false, "JPEG Image", 200),
                e("docs", true, "File Folder", 900),
            ],
            ..Default::default()
        };
        let names =
            |sv: &ShellView| -> Vec<String> { sv.entries.iter().map(|e| e.name.clone()).collect() };
        sort(&mut sv);
        assert_eq!(names(&sv), ["notes", "pics", "a.jpg", "docs"], "no sort");
        sv.sort = Some((2, true));
        sort(&mut sv);
        assert_eq!(names(&sv), ["docs", "pics", "a.jpg", "notes"], "Type");
        sv.sort = Some((3, true));
        sort(&mut sv);
        assert_eq!(names(&sv), ["pics", "docs", "notes", "a.jpg"], "Date");
        sv.sort = Some((3, false));
        sort(&mut sv);
        assert_eq!(
            names(&sv),
            ["a.jpg", "notes", "docs", "pics"],
            "Date, reversed"
        );
    }

    /// A shortcut to a folder sorts among the files, by name and type alike.
    #[test]
    fn a_shortcut_to_a_folder_sorts_among_the_files() {
        let e = |name: &str, is_dir: bool, link: bool, kind: &str| Entry {
            name: name.into(),
            is_dir,
            link,
            kind: kind.into(),
            ..Default::default()
        };
        let mut sv = ShellView {
            entries: vec![
                e("c", false, false, "Text Document"),
                e("b link", true, true, "Shortcut"),
                e("a", false, false, "Text Document"),
                e("mfolder", true, false, "File Folder"),
            ],
            sort: Some((0, true)),
            ..Default::default()
        };
        let names =
            |sv: &ShellView| -> Vec<String> { sv.entries.iter().map(|e| e.name.clone()).collect() };
        sort(&mut sv);
        assert_eq!(names(&sv), ["mfolder", "a", "b link", "c"], "Name");
        sv.sort = Some((2, true));
        sort(&mut sv);
        assert_eq!(names(&sv), ["mfolder", "b link", "a", "c"], "Type");
    }
}
