//! The Open and Save As dialog. Unlike other dialogs in this crate it does
//! not compose into [`form::Form`]: it owns its focus order and client area,
//! and a host wires `key`/`hit`/`click`/`release` directly.
//!
//! No file system access: a click that wants a folder emits
//! [`Event::NeedListing`] with the host's opaque [`FolderId`] (or
//! [`Event::NeedPlace`] with a places tile index); the host answers with
//! [`FileDlg::set_listing`].
//!
//! The client opens at `CLIENT` and resizes no narrower; a host window held shorter than
//! that takes the height out of the shell view (see `set_size`).
//! Folders list by Name ascending; a Details column click re-sorts
//! (`shellview::sort_by_column`) and the sort carries to the next folder.
//! Hosts keep the view and sort for the next dialog ([`Remembered`]).
//! The Views popup is a [`menu`](super::menu) context menu with a radio
//! bullet on the current mode.

use crate::art::viewsicon;
use crate::classic::widgets::BUTTON_W;
use crate::controls::combo::{self, Combo};
use crate::controls::form;
use crate::controls::menu::MenuLayout;
use crate::controls::metrics::ControlMetrics;
use crate::controls::placesbar::{self, Place, PlacesBar};
use crate::controls::shellview::{self, Mode, ShellView};
use crate::controls::toolbar::{self, ToolBar};
use crate::pointer::Nav;
use crate::raster::{Canvas, Rect};
use crate::selection::Mods;
use crate::text::{access_key, split_mnemonic};
use crate::textedit::TextEdit;
use crate::theme::TextUnits;
use std::borrow::Cow;
use std::cell::Cell;

mod access_tree;
mod drag;
mod draw;
mod keys;
mod layout;
mod pointer;
mod popped;
mod remembered;

pub use popped::Popped;
pub use remembered::Remembered;

/// A host-assigned folder handle, opaque to this module.
pub type FolderId = u64;

/// The pattern a Files of type label carries in its closing parentheses:
/// `*.txt` in "Text Documents (*.txt)", `*.*` in "All Files (*.*)".
/// None for a label without one ("Programs"), whose listing is the
/// host's own business.
pub fn type_pattern(label: &str) -> Option<&str> {
    let inner = label.trim_end().strip_suffix(')')?;
    let inner = &inner[inner.rfind('(')? + 1..];
    inner.contains(['*', '?']).then_some(inner)
}

/// Whether `pattern` keeps a file named `name`: any of its `;`-separated
/// globs (`crate::glob`, case folded) matches the whole name. `*.*`
/// keeps every name, with an extension or without.
pub fn pattern_keeps(pattern: &str, name: &str) -> bool {
    pattern
        .split(';')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .any(|p| p == "*.*" || crate::glob::matches(p, name))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Crumb {
    pub id: FolderId,
    pub label: String,
    pub depth: usize,
    pub icon: Option<Canvas>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Folder {
    pub id: FolderId,
    pub parent: Option<FolderId>,
    pub crumbs: Vec<Crumb>,
    pub entries: Vec<shellview::Entry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DlgMode {
    Open,
    Save,
    /// The host lists folders only, and Accept picks one.
    Folder,
}

/// Per-dialog settings; `title` is carried for the host's caption.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fields {
    pub title: String,
    pub accept_label: String,
    pub filters: Vec<String>,
    pub mode: DlgMode,
    /// Ctrl and Shift select several files. Honoured in `Open` only.
    pub multiple: bool,
}

/// The client area at its opening size (563x477 frame).
pub const CLIENT: (i32, i32) = (555, 451);

const MARGIN: i32 = 6;
const LOOKIN_LABEL: &str = "Look &in:";
const FILENAME_LABEL: &str = "File &name:";
const FILETYPE_LABEL: &str = "Files of &type:";
const CANCEL: &str = "Cancel";
/// [`FileDlg::labels`]: each label's text with its access key.
struct Labels<'a> {
    lookin: Cow<'static, str>,
    filename: Cow<'static, str>,
    filetype: Cow<'static, str>,
    accept: Cow<'a, str>,
}

/// `label` with an access key outside `taken`: its own when free, else its first letter not taken,
/// else none.
fn key_around<'a>(label: impl Into<Cow<'a, str>>, taken: &[char]) -> Cow<'a, str> {
    let label = label.into();
    let free = |c: char| c.is_alphanumeric() && c.to_lowercase().all(|l| !taken.contains(&l));
    if access_key(&label).is_some_and(free) {
        return label;
    }
    let (plain, _) = split_mnemonic(&label);
    let mut out = String::with_capacity(plain.len() + 2);
    let mut keyed = false;
    for c in plain.chars() {
        if !keyed && free(c) {
            out.push('&');
            keyed = true;
        }
        if c == '&' {
            out.push('&');
        }
        out.push(c);
    }
    if !keyed && !label.contains('&') {
        return label;
    }
    out.into()
}

/// Label text x: "Look in:" at 55, "File name:" and "Files of type:" at 102.
const LOOKIN_LABEL_X: i32 = 55;
const ROW_LABEL_X: i32 = 102;
/// How far the File name label's text box sits below its field's top;
/// the Look in and Files of type labels sit 1 and 2 px higher.
const LABEL_DY: i32 = 5;
const LOOKIN_X: i32 = 99;
const LOOKIN_Y: i32 = 8;
const LOOKIN_H: i32 = 22;
/// The toolbar starts 12 px right of the combo and its four buttons end
/// 84 px short of the client's right edge, which stays empty.
const TOOLBAR_GAP: i32 = 12;
const TOOLBAR_MARGIN: i32 = 84;
const TOOLBAR_Y: i32 = 8;
const SHELL_Y: i32 = 37;

/// The places bar runs past the shell view down to the Files of type row's bottom.
const PLACES_H: i32 = 403;
const GAP_COL: i32 = 6;
const SHELL_X: i32 = MARGIN + placesbar::WIDTH + GAP_COL;
const GAP_ROW: i32 = 9;
/// The File name row: a 22 px field from x 195 with Open 33 px past its
/// right end. The Files of type row: a 21 px combo, Cancel 2 px above
/// its top.
const FILENAME_Y: i32 = 394;
const FILENAME_H: i32 = 22;
const FILETYPE_Y: i32 = 422;
const FILETYPE_H: i32 = 21;
pub const FIELD_X: i32 = 195;
const FIELD_BUTTON_GAP: i32 = 33;
const CANCEL_DY: i32 = -2;
const GRIP: i32 = 16;
/// Rows an extra combo's open list shows at least; the client grows to fit.
const EXTRA_LIST_ROWS: usize = 4;

/// The top of a host's extra rows (`FileDlg::set_extra`), 8 px under the
/// Files of type field, in client coordinates.
pub const EXTRA_Y: i32 = FILETYPE_Y + FILETYPE_H + 8;

fn rows(r: Rect, cm: &ControlMetrics) -> Rect {
    Rect::new(r.x, r.y + cm.file_dlg.dy, r.width, r.height)
}

/// The File name and Files of type fields' width in a client `client_w`
/// wide: an extra row lines up with them from `FIELD_X`.
pub fn field_w(client_w: i32) -> i32 {
    client_w - MARGIN - BUTTON_W - FIELD_BUTTON_GAP - FIELD_X
}

/// How far an extra row may run from `FIELD_X` in a client `client_w` wide: to the right
/// margin, under the buttons.
pub fn extra_w(client_w: i32) -> i32 {
    client_w - MARGIN - FIELD_X
}

const SHELL_H: i32 = FILENAME_Y - GAP_ROW - SHELL_Y;
/// The Look in field's icon (the current crumb's) sits 6 px in and 3
/// down; the name starts 4 px past it.
const LOOKIN_ICON_X: i32 = 6;
const LOOKIN_ICON_Y: i32 = 3;
const LOOKIN_ICON_GAP: i32 = 3;
const LOOKIN_ICON_PX: i32 = 16;

/// The Look in field's folder icon slot in `field`, grown with the text by `u`.
fn lookin_icon(field: Rect, u: TextUnits) -> Rect {
    let side = u.len(LOOKIN_ICON_PX);
    let y = field.y + u.len(LOOKIN_ICON_Y) + (field.height - u.len(LOOKIN_H)) / 2;
    Rect::new(field.x + u.len(LOOKIN_ICON_X), y, side, side)
}

fn views_items() -> [(&'static str, Mode); 5] {
    [
        ("Thumbnails", Mode::Thumbnails),
        ("Tiles", Mode::Tiles),
        ("Icons", Mode::Icons),
        ("List", Mode::List),
        ("Details", Mode::Details),
    ]
}

const TOOLBAR_BACK: usize = 0;
const TOOLBAR_UP: usize = 1;
const TOOLBAR_NEW_FOLDER: usize = 2;
const TOOLBAR_VIEWS: usize = 3;

/// What a point inside `CLIENT` landed on; `click` turns it into an [`Event`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    FileName,
    FileType(combo::Hit),
    Accept,
    Cancel,
    Places(usize),
    Shell(shellview::Hit),
    LookIn(combo::Hit),
    ToolBar(usize),
    ViewsItem(usize),
    Extra(form::Hit),
    Grip,
}

/// What happened after a key or a click. `Redraw` is a change to this
/// control's own pixels only.
#[derive(Debug)]
pub enum Event {
    None,
    Redraw,
    NeedListing(FolderId),
    /// The places bar tile at this index, in the order `new` took them.
    NeedPlace(usize),
    CreateFolder(String),
    /// The names to accept: one typed or selected name, several quoted
    /// ones in the File name field's order, or `.` for the open folder in
    /// folder mode.
    Accept(Vec<String>),
    /// Files of type moved to this filter; the host lists the folder again.
    /// A pattern typed into File name stops filtering (see `Wildcard`).
    FilterChanged(usize),
    /// File name held a pattern (`*` or `?` in its last part): `name` is the
    /// text typed, `dir` its folder part as typed (`/etc/`, `sub/`), empty
    /// for the open folder. The host calls `FileDlg::apply_wildcard` once
    /// `dir` is a folder; otherwise it handles this like `Accept(vec![name])`.
    Wildcard {
        dir: String,
        name: String,
    },
    /// The size grip was pressed: the host resizes its window from the
    /// lower-right corner, and hands the new size to `set_size`.
    Resize,
    Cancel,
}

/// The names in a File name field: each `"quoted"` run when the field
/// starts with a quote, else the whole text as one name (spaces kept),
/// else none for a blank field.
pub fn split_names(field: &str) -> Vec<String> {
    if field.trim_start().starts_with('"') {
        field
            .split('"')
            .skip(1)
            .step_by(2)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    } else if field.trim().is_empty() {
        Vec::new()
    } else {
        vec![field.to_string()]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
    FileName,
    FileType,
    Extra,
    Accept,
    Cancel,
    Places,
    Shell,
    LookIn,
}

const FOCUS_ORDER: [Focus; 8] = [
    Focus::FileName,
    Focus::FileType,
    Focus::Extra,
    Focus::Accept,
    Focus::Cancel,
    Focus::Places,
    Focus::Shell,
    Focus::LookIn,
];

/// The dialog's geometry inside a caller-given rect, rebuilt by `hit` and
/// `draw` rather than cached on the model. `key` and `click` lay out only
/// the shell view.
struct Layout {
    lookin_label: Rect,
    lookin: combo::Layout,
    toolbar: Rect,
    toolbar_rects: Vec<Rect>,
    places: Rect,
    places_rects: Vec<Rect>,
    filename_label: Rect,
    filename: Rect,
    accept: Rect,
    filetype_label: Rect,
    filetype: combo::Layout,
    /// Where the Files of type list shows unless a host put it elsewhere.
    filetype_home: (i32, i32),
    cancel: Rect,
    views: Option<(Rect, MenuLayout)>,
    shell: shellview::Layout,
    grip: Rect,
}

/// The Open/Save As file dialog; owns its focus order and history stack.
#[derive(Clone, Debug, PartialEq)]
pub struct FileDlg {
    fields: Fields,
    places: PlacesBar,
    toolbar: ToolBar,
    lookin: Combo,
    shell: ShellView,
    filename: TextEdit,
    filetype: Combo,
    folder: Option<FolderId>,
    parent: Option<FolderId>,
    crumbs: Vec<Crumb>,
    history: Vec<FolderId>,
    /// The folders Back left, newest last; a folder entered any other
    /// way clears them.
    forward: Vec<FolderId>,
    /// Set by `nav` so the next `set_listing` files the folder it leaves
    /// on the other stack.
    pending: Option<Nav>,
    focus: Focus,
    views_open: bool,
    /// The Views menu's corner where a host showing it on a popup put it.
    views_at: Option<(i32, i32)>,
    /// A host shows the Look in and Files of type lists and the Views menu on popups
    /// ([`Popped`]): `draw` leaves them out.
    hosted: bool,
    extra: Option<(form::Form, i32)>,
    /// The client size the host's window has now, no narrower than
    /// `min_client`; see `set_size`.
    size: (i32, i32),
    /// The File name text the user typed or the host set, as opposed to
    /// a selection shown in the field: it comes back when the selection
    /// that replaced it is entered as a folder.
    typed: String,
    /// The pattern typed into File name that filters the view in place
    /// of Files of type, until another type is picked.
    wildcard: Option<String>,
    /// A drag resting on File name: its logical x, and the first byte shown while it rests.
    name_drop: Option<(i32, usize)>,
    /// The look's control sizes of the last `draw`, which `hit`, `access`,
    /// the keys and the clicks lay the dialog out with, and whether that
    /// look lights what the pointer rests on.
    drawn: Drawn,
    /// How its 100% layout grows with the text ([`FileDlg::set_text`]).
    text: TextUnits,
}

/// The sizes a `FileDlg` was last drawn with, and whether its look lights
/// the pointer's part. Not dialog state: it compares equal and
/// debug-prints as nothing.
#[derive(Clone, Default)]
struct Drawn(Cell<ControlMetrics>, Cell<bool>);

impl PartialEq for Drawn {
    fn eq(&self, _: &Drawn) -> bool {
        true
    }
}

impl std::fmt::Debug for Drawn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Drawn")
    }
}

impl FileDlg {
    /// The host calls `set_listing` before the first draw.
    pub fn new(fields: Fields, places: Vec<Place>) -> FileDlg {
        let filter_items: Vec<&str> = fields.filters.iter().map(String::as_str).collect();
        // The list opens above the field (see `layout`): show no more rows than
        // fit between the field and the client's top, and scroll past them.
        let mut filetype = Combo::new(&filter_items, 0, filter_items.len().max(1));
        let fit = ((FILETYPE_Y - 2) / combo::SHELL_ROW_H).max(1) as usize;
        filetype.rows = filetype.rows.min(fit);
        FileDlg {
            toolbar: ToolBar {
                items: vec![
                    // Back is a plain 23 px button here: no drop arrow
                    // on it even with history behind it.
                    toolbar::Item::Button {
                        id: toolbar::BACK,
                        icon: None,
                        label: None,
                        enabled: false,
                    },
                    toolbar::Item::Button {
                        id: toolbar::UP,
                        icon: None,
                        label: None,
                        enabled: false,
                    },
                    toolbar::Item::Button {
                        id: toolbar::NEW_FOLDER,
                        icon: None,
                        label: None,
                        enabled: true,
                    },
                    toolbar::Item::DropButton {
                        id: toolbar::VIEWS,
                        icon: Some(viewsicon::draw16()),
                        label: None,
                        enabled: true,
                    },
                ],
                hot: None,
                pressed: None,
                latched: None,
                big: false,
            },
            lookin: Combo::new(&[], 0, 8),
            shell: ShellView {
                mode: Mode::List,
                sort: Some((0, true)),
                ..Default::default()
            },
            filename: TextEdit::new("", false),
            filetype,
            places: PlacesBar {
                places,
                ..PlacesBar::default()
            },
            folder: None,
            parent: None,
            crumbs: vec![],
            history: vec![],
            forward: vec![],
            pending: None,
            focus: Focus::FileName,
            views_open: false,
            views_at: None,
            hosted: false,
            extra: None,
            size: CLIENT,
            typed: String::new(),
            wildcard: None,
            name_drop: None,
            drawn: Drawn::default(),
            text: TextUnits::ONE,
            fields,
        }
    }

    /// Replaces the listing, pushing the outgoing folder onto history (onto
    /// forward after Back; nothing for a relist) and resetting selection
    /// and scroll.
    pub fn set_listing(&mut self, folder: Folder) {
        if let Some(prev) = self.folder
            && prev != folder.id
        {
            match self.pending {
                Some(Nav::Back) => self.forward.push(prev),
                Some(Nav::Forward) => self.history.push(prev),
                None => {
                    self.history.push(prev);
                    self.forward.clear();
                }
            }
        }
        self.pending = None;
        self.parent = folder.parent;
        let items: Vec<combo::ListItem> = folder
            .crumbs
            .iter()
            .map(|c| combo::ListItem::shell(&c.label, c.icon.clone(), c.depth as i32))
            .collect();
        let selected = folder.crumbs.len().saturating_sub(1);
        self.lookin = Combo::with_items(items, selected, 8);
        self.crumbs = folder.crumbs;
        self.folder = Some(folder.id);
        self.shell.entries = folder.entries;
        shellview::sort(&mut self.shell);
        // Folder mode starts with nothing selected, so Accept picks the
        // folder just opened rather than its first subfolder.
        self.shell.selection.set_only(
            if self.shell.entries.is_empty() || self.fields.mode == DlgMode::Folder {
                None
            } else {
                Some(0)
            },
        );
        self.shell.left = 0;
        self.shell.top = 0;
        self.sync_toolbar();
    }

    /// Host extra rows of `h` px under the Files of type row, laid out from
    /// `(FIELD_X, EXTRA_Y)`; the client grows by `h`. Each combo's open list
    /// must fit inside the client: `h` grows to show up to `EXTRA_LIST_ROWS`
    /// rows, and longer lists are shortened and scroll.
    pub fn set_extra(&mut self, mut form: form::Form, h: i32, cm: &ControlMetrics) {
        let mut h = h;
        for it in &mut form.items {
            let field = it.rect;
            if let form::Control::Combo(c) = &mut it.control {
                let want = c.rows.min(EXTRA_LIST_ROWS);
                while c.rows > want && combo::list_rect(c, field, cm).bottom() > CLIENT.1 + h {
                    c.rows -= 1;
                }
                h = h.max(combo::list_rect(c, field, cm).bottom() - CLIENT.1);
            }
        }
        self.extra = Some((form, h));
        self.grow_to_min();
    }

    pub fn extra(&self) -> Option<&form::Form> {
        self.extra.as_ref().map(|(f, _)| f)
    }

    /// The client size the dialog opens at and a host's window minimum:
    /// `CLIENT` with the extra rows.
    pub fn min_client(&self) -> (i32, i32) {
        let u = self.text;
        (
            u.len(CLIENT.0),
            u.len(CLIENT.1 + self.extra.as_ref().map_or(0, |(_, h)| *h)),
        )
    }

    /// Lay the dialog out for text grown by `u` from here on: the host draws it with a theme of
    /// the same [`Theme::text`]. A client at its minimum takes the new minimum; a larger one
    /// keeps its size, no smaller than the new minimum.
    pub fn set_text(&mut self, u: TextUnits) {
        let at_min = self.size == self.min_client();
        self.text = u;
        if at_min {
            self.size = self.min_client();
        }
        self.grow_to_min();
    }

    /// The text size the dialog is laid out for ([`FileDlg::set_text`]).
    pub fn text(&self) -> TextUnits {
        self.text
    }

    fn grow_to_min(&mut self) {
        let min = self.min_client();
        self.size = (self.size.0.max(min.0), self.size.1.max(min.1));
    }

    pub fn client(&self) -> (i32, i32) {
        self.size
    }

    /// The host window's client size. A width under `min_client`'s is taken as that; a
    /// height under it (an output too short for the dialog) is laid out by `grow`.
    pub fn set_size(&mut self, size: (i32, i32)) {
        self.size = (size.0.max(self.min_client().0), size.1);
    }

    /// How far `r` is taller than `min_client`: the shell view and the places bar grow by
    /// it, and every row under them moves down by it. A shorter `r` shrinks them by up to
    /// `shrink`, so the rows under them stay inside it.
    fn grow(&self, r: Rect, cm: &ControlMetrics) -> i32 {
        (r.height - self.min_client().1).max(-self.shrink(cm))
    }

    /// How far the dialog lays out shorter than `min_client`: the places bar down to its
    /// last tile at its least ([`PlacesMetrics::least_tiles`]), the shell view down to half its
    /// opening height.
    ///
    /// [`PlacesMetrics::least_tiles`]: crate::metrics::PlacesMetrics::least_tiles
    fn shrink(&self, cm: &ControlMetrics) -> i32 {
        let u = |v: i32| cm.text.len(v);
        let tiles = cm.places.least_tiles(self.places.places.len());
        let places = u(SHELL_Y + PLACES_H) - u(SHELL_Y) - tiles;
        let shell = (u(SHELL_Y + SHELL_H) - u(SHELL_Y)) / 2;
        places.min(shell).max(0)
    }

    fn extra_list_open(&self) -> bool {
        self.extra.as_ref().is_some_and(|(f, _)| {
            f.items
                .iter()
                .any(|it| matches!(&it.control, form::Control::Combo(c) if c.open.is_some()))
        })
    }

    fn multi(&self) -> bool {
        self.fields.multiple && self.fields.mode == DlgMode::Open
    }

    /// Puts the selection in the File name field: several files quoted
    /// (from the focused entry, wrapping), one file or a lone folder by
    /// name, nothing selected the typed text.
    fn sync_name_field(&mut self) {
        let set = &self.shell.selection.set;
        let from = self.shell.selection.focus.unwrap_or(0);
        let sel: Vec<&shellview::Entry> = set
            .range(from..)
            .chain(set.range(..from))
            .filter_map(|&i| self.shell.entries.get(i))
            .collect();
        let files: Vec<&str> = sel
            .iter()
            .filter(|e| !e.is_dir)
            .map(|e| e.name.as_str())
            .collect();
        let text = if self.multi() && files.len() >= 2 {
            let quoted: Vec<String> = files.iter().map(|n| format!("\"{n}\"")).collect();
            quoted.join(" ")
        } else if let [one] = files[..] {
            one.to_string()
        } else if let [one] = sel[..] {
            one.name.clone()
        } else {
            self.typed.clone()
        };
        self.filename.set_selected(text);
    }

    /// A folder is being entered: the field goes back to what was typed,
    /// or empties when what was typed is the folder's own name.
    fn entering(&mut self, name: &str) {
        if self.typed == name || self.fields.mode == DlgMode::Folder {
            self.typed.clear();
        }
        self.filename.set_selected(self.typed.clone());
    }

    /// The host opened a folder the user typed a path to: the field
    /// empties so Accept does not take the path again.
    pub fn clear_file_name(&mut self) {
        self.typed.clear();
        self.filename.set_selected("");
    }

    fn sel_mods(&self, mods: Mods) -> Mods {
        if self.multi() { mods } else { Mods::default() }
    }

    pub fn can_go_up(&self) -> bool {
        self.parent.is_some()
    }

    /// Set the toolbar icons (Back, Up, New Folder, Views); `None` clears a
    /// slot, except Views, which falls back to `viewsicon::draw16`.
    pub fn set_toolbar_icons(
        &mut self,
        back: Option<Canvas>,
        up: Option<Canvas>,
        new_folder: Option<Canvas>,
        views: Option<Canvas>,
    ) {
        for (item, new_icon) in self.toolbar.items.iter_mut().zip([
            back,
            up,
            new_folder,
            views.or_else(|| Some(viewsicon::draw16())),
        ]) {
            match item {
                toolbar::Item::Button { icon, .. } | toolbar::Item::DropButton { icon, .. } => {
                    *icon = new_icon;
                }
                toolbar::Item::Separator => {}
            }
        }
    }

    /// Replace the places tiles' icons, in the order `new` took the tiles.
    pub fn set_place_icons(&mut self, icons: Vec<Option<Canvas>>) {
        for (place, icon) in self.places.places.iter_mut().zip(icons) {
            place.icon = icon;
        }
    }

    /// Replace the Look in folders' icons, in the listing's crumb order.
    pub fn set_crumb_icons(&mut self, icons: Vec<Option<Canvas>>) {
        for ((crumb, item), icon) in self
            .crumbs
            .iter_mut()
            .zip(self.lookin.items.iter_mut())
            .zip(icons)
        {
            item.icon = icon.clone();
            crumb.icon = icon;
        }
    }

    fn sync_toolbar(&mut self) {
        if let toolbar::Item::Button { enabled, .. } = &mut self.toolbar.items[TOOLBAR_BACK] {
            *enabled = !self.history.is_empty();
        }
        if let toolbar::Item::Button { enabled, .. } = &mut self.toolbar.items[TOOLBAR_UP] {
            *enabled = self.parent.is_some();
        }
    }

    /// Pops the Back (or Forward) stack and asks for that folder; `None`
    /// when there is nowhere to go.
    pub fn nav(&mut self, nav: Nav) -> Event {
        let stack = match nav {
            Nav::Back => &mut self.history,
            Nav::Forward => &mut self.forward,
        };
        match stack.pop() {
            Some(id) => {
                self.pending = Some(nav);
                self.sync_toolbar();
                Event::NeedListing(id)
            }
            None => Event::None,
        }
    }

    fn up(&mut self) -> Event {
        match self.parent {
            Some(id) => Event::NeedListing(id),
            None => Event::None,
        }
    }

    pub fn mode(&self) -> DlgMode {
        self.fields.mode
    }

    pub fn folder(&self) -> Option<FolderId> {
        self.folder
    }

    pub fn file_name(&self) -> &str {
        &self.filename.text
    }

    pub fn focused_edit(&self) -> Option<&TextEdit> {
        (self.focus == Focus::FileName).then_some(&self.filename)
    }

    pub fn entries(&self) -> &[shellview::Entry] {
        &self.shell.entries
    }

    /// For a host filling in what arrives after the listing, such as a picture's thumbnail.
    pub fn entries_mut(&mut self) -> &mut [shellview::Entry] {
        &mut self.shell.entries
    }

    /// The view picked from Views.
    pub fn view(&self) -> Mode {
        self.shell.mode
    }

    /// The view and sort to hand the next dialog. A view with no sort column reads as Name.
    pub fn remembered(&self) -> Remembered {
        let (col, ascending) = self.shell.sort.unwrap_or((0, true));
        match self.shell.fields.get(col) {
            Some(&sort) => Remembered {
                mode: self.shell.mode,
                sort,
                ascending,
            },
            None => Remembered {
                mode: self.shell.mode,
                ..Remembered::default()
            },
        }
    }

    /// Opens in `r`'s view and sort; a sort column this dialog does not show sorts by Name
    /// ascending.
    pub fn restore(&mut self, r: Remembered) {
        self.shell.set_mode(r.mode);
        self.shell.sort = Some(match self.shell.fields.iter().position(|&f| f == r.sort) {
            Some(col) => (col, r.ascending),
            None => (0, true),
        });
        shellview::sort(&mut self.shell);
    }

    pub fn set_filter(&mut self, i: usize) {
        self.filetype.selected = i.min(self.fields.filters.len().saturating_sub(1));
    }

    pub fn filter(&self) -> usize {
        self.filetype.selected
    }

    /// Makes the last part of `name` (after any `/`) the view's filter and
    /// the File name text (`/etc/*.conf` leaves `*.conf`). A host calls it
    /// for an `Event::Wildcard` whose folder part is a folder.
    pub fn apply_wildcard(&mut self, name: &str) {
        let pattern = name.rsplit('/').next().unwrap_or(name);
        self.wildcard = Some(pattern.to_string());
        self.typed = pattern.to_string();
        self.filename.set_selected(pattern);
    }

    /// The pattern typed into File name that filters the view, if any:
    /// a host lists the files it keeps (`pattern_keeps`) and every
    /// folder, whatever Files of type says.
    pub fn wildcard(&self) -> Option<&str> {
        self.wildcard.as_deref()
    }

    /// The picked Files of type row's `type_pattern`, owned so a host's
    /// keep can hold it while it lists into the dialog.
    pub fn picked_pattern(&self) -> Option<String> {
        let label = self.fields.filters.get(self.filetype.selected)?;
        type_pattern(label).map(str::to_string)
    }

    fn crumb_icon(&self) -> Option<&Canvas> {
        self.crumbs.get(self.lookin.selected)?.icon.as_ref()
    }

    pub fn set_file_name(&mut self, name: &str) {
        self.filename.set_selected(name);
        self.typed = name.to_string();
    }
}

#[cfg(test)]
mod tests;
