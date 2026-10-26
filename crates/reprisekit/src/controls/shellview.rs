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

use crate::classic::widgets::{Axis, BarHit, ScrollHit};
use crate::controls::iconview::Cell;
use crate::controls::listview;
use crate::controls::metrics::ControlMetrics;
use crate::icontheme::ListingSizes;
use crate::look::widgets::item::Mark;
use crate::pointer::Pointer;
use crate::raster::{Canvas, Rect};
use crate::selection::Selection;
use crate::textedit::TextEdit;
use crate::theme::{Color, TextUnits};
use std::collections::HashMap;
use std::rc::Rc;

mod access;
mod details;
mod drag;
mod draw;
mod hit;
mod keys;
mod layout;
mod rename;
mod scroll;

pub use access::access;
pub use details::{
    DetailsRows, Field, bin_columns, bin_fields, details_columns, details_fields, resize_column,
    search_columns, search_fields, sort, sort_by_column,
};
pub use drag::{DragRow, drag_image};
pub use draw::draw;
pub use hit::{hit, name_hit};
pub use keys::{step, type_ahead};
#[cfg(test)]
pub(crate) use layout::LAYOUTS;
pub use layout::{LayoutCache, label_rect, layout};
pub(crate) use rename::draw_rename;
pub use rename::{edit_index_at, rename_rect};
pub use scroll::{
    drag_hthumb, drag_thumb, ensure_visible, follow, hpage, hscroll, hstep, page, reveal, scroll,
};

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
/// for List and Details (16 px, or [`list_icon_px`] grown with the text), `icon32` for Icons
/// and for List and Details once the text grows their icon column to 32 px ([`list_icon`]),
/// `icon48` for Tiles, `thumb` for Thumbnails. `id` is the host's own handle (a `filedlg::FolderId`
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

/// Entries wearing a pair's old icons wear its new ones, as
/// [`ListingIcons::refetch`](crate::icontheme::ListingIcons::refetch) and
/// [`put`](crate::icontheme::ListingIcons::put) give them; the rest keep theirs.
pub fn swap_icons(entries: &mut [Entry], swaps: &[(ListingSizes, ListingSizes)]) {
    let first = |s: [&Option<Rc<Canvas>>; 3]| s.into_iter().flatten().next().map(Rc::as_ptr);
    let new: HashMap<*const Canvas, &ListingSizes> = swaps
        .iter()
        .filter_map(|(old, new)| Some((first([&old[0], &old[1], &old[2]])?, new)))
        .collect();
    for e in entries {
        if let Some(n) = first([&e.icon, &e.icon32, &e.icon48]).and_then(|p| new.get(&p)) {
            [e.icon, e.icon32, e.icon48] = (*n).clone();
        }
    }
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

/// The device pixels [`THUMB_INNER`] covers at `scale`: the size a thumbnail is fetched at.
pub fn thumb_px(scale: i32) -> u32 {
    THUMB_INNER * scale.max(1) as u32
}

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

/// List and Details' icon column at text size `u`, the size a host fetches `Entry::icon` at.
pub fn list_icon_px(u: TextUnits) -> i32 {
    u.len(ICON_PX)
}

/// The icon List and Details draw for `e` in an icon column `slot` px wide: the 32 px one once
/// it fits, else `icon`.
pub fn list_icon(e: &Entry, slot: i32) -> Option<&std::rc::Rc<Canvas>> {
    match slot >= 32 {
        true => e.icon32.as_ref().or(e.icon.as_ref()),
        false => e.icon.as_ref(),
    }
}

/// The [`ListingIcons::sizes`](crate::icontheme::ListingIcons::sizes) `mode` draws an entry's
/// icon at, with List and Details' icon column `list` px wide ([`list_icon`]).
pub fn drawn_sizes(mode: Mode, list: i32) -> [bool; 3] {
    match mode {
        Mode::List | Mode::Details => [true, list >= 32, false],
        Mode::Icons | Mode::Thumbnails => [false, true, false],
        Mode::Tiles => [false, false, true],
    }
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

#[cfg(test)]
pub(crate) mod tests;
