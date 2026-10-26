//! Details' columns and fields, sorting by a column, and the transient `listview` Details runs through.

use super::{Entry, ICON_PX, Layout, ShellView, list_icon, size_text};
use crate::controls::listview;
use crate::raster::Canvas;
use crate::selection::Selection;
use crate::theme::TextUnits;
use std::borrow::Cow;

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

/// Search results' five columns: Name 178, In Folder 180, Size 94 (right-aligned), Type 118,
/// Date Modified 120.
pub fn search_columns() -> Vec<listview::Column> {
    let mut cols = details_columns();
    cols.insert(1, listview::Column::new("In Folder", 180));
    cols
}

pub fn search_fields() -> Vec<Field> {
    vec![
        Field::Name,
        Field::Folder,
        Field::Size,
        Field::Kind,
        Field::Modified,
    ]
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
    fn icon_slot(&self, u: TextUnits) -> i32 {
        u.len(ICON_PX)
    }
    fn icon_in(&self, row: usize, slot: i32) -> Option<&Canvas> {
        self.0
            .get(row)
            .and_then(|e| list_icon(e, slot))
            .map(|c| &**c)
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

pub(super) fn details_listview(sv: &ShellView) -> DetailsView<'_> {
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

/// Details' transient view of `sv` and its layout in `lay`.
pub(super) fn details_in<'a>(
    sv: &'a ShellView,
    lay: &Layout,
) -> (DetailsView<'a>, listview::Layout) {
    let lv = details_listview(sv);
    let lv_lay = listview::layout(&lv, lay.frame, &lay.controls);
    (lv, lv_lay)
}

/// Run `f` on a Details `ListView` that owns `sv`'s columns, selection and rename for the
/// call (moved out, not copied) and on its layout in `lay`, then move changes back into `sv`.
pub(super) fn with_details<T>(
    sv: &mut ShellView,
    lay: &Layout,
    f: impl FnOnce(&mut listview::ListView<DetailsRows<'_>>, &listview::Layout) -> T,
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
    let lv_lay = listview::layout(&lv, lay.frame, &lay.controls);
    let out = f(&mut lv, &lv_lay);
    *columns = lv.columns;
    *selection = lv.selection;
    *top = lv.top;
    *left = lv.left;
    *focus_ring = lv.focus_ring;
    *edit = lv.edit;
    out
}
