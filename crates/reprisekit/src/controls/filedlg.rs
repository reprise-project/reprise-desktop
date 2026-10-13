//! The Open and Save As dialog. Unlike other dialogs in this crate it does
//! not compose into [`form::Form`]: it owns its focus order and client area,
//! and a host wires `key`/`hit`/`click`/`release` directly.
//!
//! No file system access: a click that wants a folder emits
//! [`Event::NeedListing`] with the host's opaque [`FolderId`] (or
//! [`Event::NeedPlace`] with a places tile index); the host answers with
//! [`FileDlg::set_listing`].
//!
//! The client opens at `CLIENT` and resizes no smaller (see `set_size`).
//! Folders list by Name ascending; a Details column click re-sorts
//! (`shellview::sort_by_column`) and the sort carries to the next folder.
//! The Views popup is a [`menu`](super::menu) context menu with a radio
//! bullet on the current mode.

use crate::access::{self, Tree, accesskit};
use crate::art::viewsicon;
use crate::classic::widgets::{
    self, BUTTON_H, BUTTON_W, BarHit, LINE_H, draw_edit, edit_first_visible,
};
use crate::controls::combo::{self, Combo};
use crate::controls::form;
use crate::controls::iconview::IconMove;
use crate::controls::menu::{ContextBitmaps, MenuItem, MenuLayout, MenuStyle};
use crate::controls::metrics::ControlMetrics;
use crate::controls::placesbar::{self, Place, PlacesBar};
use crate::controls::shellview::{self, Mode, ShellView};
use crate::controls::toolbar::{self, ToolBar};
use crate::look::Paint;
use crate::look::widgets::{draw_button, draw_edit_field};
use crate::pointer::{Interaction, Nav, WHEEL_ROWS};
use crate::raster::{Canvas, Rect};
use crate::selection::Mods;
use crate::text::{Align, TextRenderer};
use crate::textedit::{Caret, EditKey, TextEdit};
use crate::theme::{TextUnits, Theme};
use accesskit::NodeId;
use std::cell::Cell;

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

const SHELL_H: i32 = FILENAME_Y - GAP_ROW - SHELL_Y;
/// The Look in field's icon (the current crumb's) sits 6 px in and 3
/// down; the name starts 4 px past it.
const LOOKIN_ICON_X: i32 = 6;
const LOOKIN_ICON_Y: i32 = 3;
const LOOKIN_ICON_GAP: i32 = 3;
const LOOKIN_ICON_PX: i32 = 16;

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
    extra: Option<(form::Form, i32)>,
    /// The client size the host's window has now, no smaller than
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

fn combo_click(combo: &mut Combo, h: combo::Hit) {
    match h {
        combo::Hit::Field | combo::Hit::Drop => {
            combo.toggle();
        }
        combo::Hit::Row(i) => {
            combo.pick(i);
        }
        combo::Hit::ScrollUp => {
            combo.scroll(-1);
        }
        combo::Hit::ScrollDown => {
            combo.scroll(1);
        }
        combo::Hit::PageUp => {
            combo.scroll(-(combo.rows as i32));
        }
        combo::Hit::PageDown => {
            combo.scroll(combo.rows as i32);
        }
        combo::Hit::Thumb(_) => {}
    }
}

fn combo_key(combo: &mut Combo, key: form::Key) -> Event {
    match key {
        form::Key::Drop => {
            combo.toggle();
            Event::Redraw
        }
        form::Key::Up => {
            combo.step(false);
            Event::Redraw
        }
        form::Key::Down => {
            combo.step(true);
            Event::Redraw
        }
        form::Key::Home => {
            combo.jump(false);
            Event::Redraw
        }
        form::Key::End => {
            combo.jump(true);
            Event::Redraw
        }
        form::Key::PageUp => {
            combo.scroll(-(combo.rows as i32));
            Event::Redraw
        }
        form::Key::PageDown => {
            combo.scroll(combo.rows as i32);
            Event::Redraw
        }
        form::Key::Enter if combo.open.is_some() => {
            combo.take();
            Event::Redraw
        }
        _ => Event::None,
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
                selected: None,
            },
            folder: None,
            parent: None,
            crumbs: vec![],
            history: vec![],
            forward: vec![],
            pending: None,
            focus: Focus::FileName,
            views_open: false,
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
        self.set_size(self.size);
    }

    pub fn extra(&self) -> Option<&form::Form> {
        self.extra.as_ref().map(|(f, _)| f)
    }

    /// The client size the dialog opens at and shrinks to no further:
    /// `CLIENT` with the extra rows.
    pub fn min_client(&self) -> (i32, i32) {
        let u = self.text;
        (
            u.len(CLIENT.0),
            u.len(CLIENT.1 + self.extra.as_ref().map_or(0, |(_, h)| *h)),
        )
    }

    /// Lay the dialog out for text grown by `u` from here on: the host draws it with a theme of
    /// the same [`Theme::text`]. The client grows to the new minimum.
    pub fn set_text(&mut self, u: TextUnits) {
        self.text = u;
        self.set_size(self.size);
    }

    pub fn client(&self) -> (i32, i32) {
        self.size
    }

    pub fn set_size(&mut self, size: (i32, i32)) {
        let min = self.min_client();
        self.size = (size.0.max(min.0), size.1.max(min.1));
    }

    /// How far `r` is taller than `min_client`: the shell view and the
    /// places bar grow by it, and every row under them moves down by it.
    fn grow(&self, r: Rect) -> i32 {
        (r.height - self.min_client().1).max(0)
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

    /// A drag at (`x`, `y`) in `r`: over File name the field shows a drop caret there and takes
    /// the drag (`true`); anywhere else the drop caret goes.
    pub fn drag_over(&mut self, text: &TextRenderer, px: u32, r: Rect, x: i32, y: i32) -> bool {
        if self.views_open || self.hit(text, px, r, x, y) != Some(Hit::FileName) {
            self.name_drop = None;
            return false;
        }
        let first = match self.name_drop {
            Some((_, first)) => first,
            None => {
                let w = self.name_inner(text, px, r).width;
                edit_first_visible(text, &self.filename, w, px, 0)
            }
        };
        self.name_drop = Some((x, first));
        true
    }

    /// The drag left or ended: `true` when a drop caret was showing.
    pub fn drag_leave(&mut self) -> bool {
        self.name_drop.take().is_some()
    }

    /// The first byte shown one step on for a drag resting at File name's edge
    /// ([`crate::textedit::drag_scroll`]), None when it stays.
    pub fn drag_scroll(&self, text: &TextRenderer, px: u32, r: Rect) -> Option<usize> {
        let (x, first) = self.name_drop?;
        let inner = self.name_inner(text, px, r);
        let s = &self.filename.text;
        crate::textedit::drag_scroll(text, s, first, inner.width, LINE_H, x - inner.x, px)
    }

    /// Take [`FileDlg::drag_scroll`]'s step, for a host's tick; `true` when the field moved.
    pub fn drag_tick(&mut self, text: &TextRenderer, px: u32, r: Rect) -> bool {
        let Some(to) = self.drag_scroll(text, px, r) else {
            return false;
        };
        if let Some((_, first)) = &mut self.name_drop {
            *first = to;
        }
        true
    }

    /// A drag dropped `line` on File name: it goes in at the drop caret as
    /// [`TextEdit::drop_text`] puts it, and the field takes the focus. `false` when the field
    /// was not taking the drag or nothing went in.
    pub fn drop_text(
        &mut self,
        text: &TextRenderer,
        px: u32,
        r: Rect,
        line: &crate::textedit::DropLine,
    ) -> bool {
        let Some((x, first)) = self.name_drop.take() else {
            return false;
        };
        let inner = self.name_inner(text, px, r);
        let first = first.min(self.filename.text.len());
        let tail = &self.filename.text[first..];
        let at = first + crate::textedit::index_at(text, tail, x - inner.x, px);
        if !self.filename.drop_text(at, line) {
            return false;
        }
        self.typed = self.filename.text.clone();
        self.focus = Focus::FileName;
        true
    }

    /// Files dropped on File name replace its text, selected: `names` as the field takes a
    /// selection of several (quoted) where the dialog picks several, else the first.
    pub fn drop_names(&mut self, names: &[String]) -> bool {
        if self.name_drop.take().is_none() {
            return false;
        }
        let text = match names {
            [] => return false,
            [_, _, ..] if self.multi() => names
                .iter()
                .map(|n| format!("\"{n}\""))
                .collect::<Vec<_>>()
                .join(" "),
            [first, ..] => first.clone(),
        };
        self.set_file_name(&text);
        self.focus = Focus::FileName;
        true
    }

    /// File name's text area in `r`, in logical px.
    fn name_inner(&self, text: &TextRenderer, px: u32, r: Rect) -> Rect {
        let lay = self.layout(text, px, r, &self.drawn.0.get());
        lay.filename.inset(widgets::FIELD_PAD)
    }

    fn layout(&self, text: &TextRenderer, px: u32, r: Rect, cm: &ControlMetrics) -> Layout {
        let client = r;
        let dy = self.grow(r);
        let r = rows(r, cm);
        // 100% lengths at the text size; the toolbar's buttons and the icons keep their px.
        let u = |v: i32| cm.text.len(v);
        let label_h = px as i32 + 2;
        let lookin_label_w = text.measure("Look in:", false, px);
        let lookin_label = Rect::new(
            r.x + u(LOOKIN_LABEL_X),
            r.y + u(LOOKIN_Y + LABEL_DY) - 1,
            lookin_label_w,
            label_h,
        );
        let lookin_h = u(LOOKIN_H);
        let toolbar_y = r.y + u(TOOLBAR_Y) + (lookin_h - LOOKIN_H) / 2;

        // An item's width does not depend on the rect, so one pass at x 0
        // gives the natural width, and the rects shift into place after it.
        let mut toolbar_rects = toolbar::layout(
            &self.toolbar,
            text,
            px,
            Rect::new(0, toolbar_y, 0, toolbar::HEIGHT),
            cm,
        );
        let toolbar_w = toolbar_rects.last().map_or(0, |b| b.right());
        let toolbar_x = r.right() - u(TOOLBAR_MARGIN) - toolbar_w;
        for b in &mut toolbar_rects {
            b.x += toolbar_x;
        }
        let toolbar = Rect::new(toolbar_x, toolbar_y, toolbar_w, toolbar::HEIGHT);

        let lookin_x = r.x + u(LOOKIN_X);
        let lookin_w = (toolbar_x - u(TOOLBAR_GAP) - lookin_x).max(0);
        let lookin_field = Rect::new(lookin_x, r.y + u(LOOKIN_Y), lookin_w, lookin_h);
        let mut lookin = combo::layout(&self.lookin, lookin_field, cm);
        if self.crumb_icon().is_some() {
            let shift = LOOKIN_ICON_X + LOOKIN_ICON_PX + LOOKIN_ICON_GAP - 2;
            lookin.text.x += shift;
            lookin.text.width -= shift;
            lookin.highlight.x += shift;
            lookin.highlight.width -= shift;
        }

        let places = Rect::new(
            r.x + u(MARGIN),
            r.y + u(SHELL_Y),
            u(MARGIN + placesbar::WIDTH) - u(MARGIN),
            u(SHELL_Y + PLACES_H) - u(SHELL_Y) + dy,
        );
        let places_rects = placesbar::layout(&self.places, places, &cm.places);

        let label_w =
            text.measure("File name:", false, px)
                .max(text.measure("Files of type:", false, px));
        let field_x = r.x + u(FIELD_X);
        let (button_w, button_h) = (u(BUTTON_W), u(BUTTON_H));
        let accept_x = r.right() - u(MARGIN) - button_w;
        let field_w = (accept_x - u(FIELD_BUTTON_GAP) - field_x).max(0);

        let filename_y = r.y + dy + u(FILENAME_Y);
        let filename_label = Rect::new(
            r.x + u(ROW_LABEL_X),
            filename_y + u(LABEL_DY),
            label_w,
            label_h,
        );
        let filename = Rect::new(field_x, filename_y, field_w, u(FILENAME_H));
        let accept = Rect::new(accept_x, filename_y, button_w, button_h);

        // This label sits 2 px higher against its field than the other
        // two (its baseline 13 px below the field's top).
        let filetype_y = r.y + dy + u(FILETYPE_Y);
        let filetype_label = Rect::new(
            r.x + u(ROW_LABEL_X),
            filetype_y + u(LABEL_DY) - 2,
            label_w,
            label_h,
        );
        let filetype_field = Rect::new(field_x, filetype_y, field_w, u(FILETYPE_H));
        // The row sits near the client's bottom edge, and a list drawn
        // past it could not be clicked: it opens above the field instead.
        let mut filetype = combo::layout(&self.filetype, filetype_field, cm);
        if filetype.list.bottom() > client.bottom() {
            let dy = filetype.field.y - filetype.list.bottom();
            filetype.list.y += dy;
            if let Some(sb) = &mut filetype.scrollbar {
                sb.y += dy;
            }
        }
        let cancel = Rect::new(accept_x, filetype_y + CANCEL_DY, button_w, button_h);

        let views = self.views_open.then(|| {
            let anchor = toolbar_rects[TOOLBAR_VIEWS];
            let menu = self.views_menu(text, px, r.width, cm.text);
            let popup = Rect::new(anchor.x, anchor.bottom(), menu.width, menu.height);
            (popup, menu)
        });

        let shell = self.shell_layout(client, cm);

        Layout {
            lookin_label,
            lookin,
            toolbar,
            toolbar_rects,
            places,
            places_rects,
            filename_label,
            filename,
            accept,
            filetype_label,
            filetype,
            cancel,
            views,
            shell,
            grip: Rect::new(client.right() - GRIP, client.bottom() - GRIP, GRIP, GRIP),
        }
    }

    fn shell_layout(&self, r: Rect, cm: &ControlMetrics) -> shellview::Layout {
        let dy = self.grow(r);
        let r = rows(r, cm);
        let u = |v: i32| cm.text.len(v);
        let shell_w = (r.width - u(SHELL_X) - u(MARGIN)).max(0);
        let shell_h = u(SHELL_Y + SHELL_H) - u(SHELL_Y) + dy;
        let shell = Rect::new(r.x + u(SHELL_X), r.y + u(SHELL_Y), shell_w, shell_h);
        shellview::layout(&self.shell, shell, cm)
    }

    fn views_menu_items(&self) -> Vec<MenuItem> {
        views_items()
            .iter()
            .map(|(label, mode)| {
                MenuItem::entry(*label)
                    .radio(true)
                    .checked(*mode == self.shell.mode)
            })
            .collect()
    }

    fn views_menu(&self, text: &TextRenderer, px: u32, max_width: i32, u: TextUnits) -> MenuLayout {
        let items = self.views_menu_items();
        let mut theme = Theme::default();
        theme.fonts.size_px = u.px(theme.fonts.size_px);
        let style =
            MenuStyle::context(&theme, ContextBitmaps::None).fit(text, &items, px, max_width);
        MenuLayout::compute_styled(&style, &items, usize::MAX)
    }

    fn advance_focus(&mut self, back: bool) {
        if self.focus == Focus::Extra
            && let Some((f, _)) = &mut self.extra
            && f.tab_no_wrap(back)
        {
            return;
        }
        self.step_focus(back);
        if self.focus == Focus::Extra {
            let entered = match &mut self.extra {
                Some((f, _)) => f.focus_edge(back),
                None => false,
            };
            if !entered {
                self.step_focus(back);
            }
        }
    }

    fn step_focus(&mut self, back: bool) {
        let i = FOCUS_ORDER
            .iter()
            .position(|f| *f == self.focus)
            .unwrap_or(0);
        let n = FOCUS_ORDER.len() as i32;
        let step = if back { -1 } else { 1 };
        let next = (i as i32 + step).rem_euclid(n);
        self.focus = FOCUS_ORDER[next as usize];
        if self.focus == Focus::FileType && !self.has_types() {
            self.step_focus(back);
        }
    }

    /// Whether the dialog has a Files of type row: not in folder mode,
    /// which lists folders only.
    fn has_types(&self) -> bool {
        self.fields.mode != DlgMode::Folder
    }

    /// The default action. A selected folder whose name is all the File
    /// name field holds opens, or, from the accept button in folder mode,
    /// is picked. Otherwise the field's names are accepted, else the
    /// focused file's, else (folder mode) the open folder as `.`.
    fn activate(&mut self, from_button: bool) -> Event {
        if self.focus == Focus::Places {
            return match self.places.selected {
                Some(i) if self.places.places.get(i).is_some_and(|p| p.enabled) => {
                    Event::NeedPlace(i)
                }
                _ => Event::None,
            };
        }
        // A selected directory wins when the field holds only its name:
        // `shell_click` writes the selection into the field, so accepting
        // the text would "open" a folder as a file. A typed name still wins.
        let folder_pick = self.fields.mode == DlgMode::Folder && from_button;
        let focused = self.shell.selection.focus;
        if let Some(entry) = focused.and_then(|i| self.shell.entries.get(i))
            && entry.is_dir
            && (self.filename.text.is_empty() || self.filename.text == entry.name)
        {
            if folder_pick {
                return Event::Accept(vec![entry.name.clone()]);
            }
            let (id, name) = (entry.id, entry.name.clone());
            self.entering(&name);
            return Event::NeedListing(id);
        }
        let names = split_names(&self.filename.text);
        if let [name] = &names[..]
            && let Some(ev) = self.take_wildcard(name)
        {
            return ev;
        }
        if !names.is_empty() {
            return Event::Accept(names);
        }
        if let Some(entry) = focused.and_then(|i| self.shell.entries.get(i)) {
            if !entry.is_dir {
                return Event::Accept(vec![entry.name.clone()]);
            }
            let (id, name) = (entry.id, entry.name.clone());
            self.entering(&name);
            return Event::NeedListing(id);
        }
        if self.fields.mode == DlgMode::Folder {
            return Event::Accept(vec![".".into()]);
        }
        Event::None
    }

    /// A File name whose last part holds `*` or `?` is a pattern, not a name
    /// to accept, unless a listed entry has that name. Folder mode takes no
    /// pattern.
    fn take_wildcard(&self, name: &str) -> Option<Event> {
        let (dir, pattern) = match name.rfind('/') {
            Some(i) => name.split_at(i + 1),
            None => ("", name),
        };
        if self.fields.mode == DlgMode::Folder
            || !pattern.contains(['*', '?'])
            || self.shell.entries.iter().any(|e| e.name == name)
        {
            return None;
        }
        Some(Event::Wildcard {
            dir: dir.to_string(),
            name: name.to_string(),
        })
    }

    /// A key while a sub-control has the focus. `ch` is the keystroke's
    /// literal character for the shell view's type-ahead; `r` is the rect
    /// `draw` was given.
    pub fn key(&mut self, r: Rect, key: form::Key, ch: Option<char>, mods: Mods) -> Event {
        let before = self.filetype.selected;
        let cm = self.drawn.0.get();
        let ev = self.key_inner(r, key, ch, mods, &cm);
        self.filter_event(before, ev)
    }

    /// `ev`, or `FilterChanged` when the Files of type pick moved, which
    /// also drops a typed pattern's filter.
    fn filter_event(&mut self, before: usize, ev: Event) -> Event {
        if self.filetype.selected != before && matches!(ev, Event::Redraw | Event::None) {
            self.wildcard = None;
            Event::FilterChanged(self.filetype.selected)
        } else {
            ev
        }
    }

    fn key_inner(
        &mut self,
        r: Rect,
        key: form::Key,
        ch: Option<char>,
        mods: Mods,
        cm: &ControlMetrics,
    ) -> Event {
        if self.views_open {
            return match key {
                form::Key::Escape | form::Key::Drop => {
                    self.views_open = false;
                    Event::Redraw
                }
                _ => Event::None,
            };
        }
        // A dropped list owns Enter and Escape: Enter commits the row (Look in
        // navigates to the crumb), Escape closes the list. Neither may reach
        // `activate` or `combo_key`.
        let combo_open = self.lookin.open.is_some() || self.filetype.open.is_some();
        if combo_open {
            if matches!(key, form::Key::Escape) {
                self.lookin.open = None;
                self.filetype.open = None;
                return Event::Redraw;
            }
            if matches!(key, form::Key::Enter) {
                if let Some(i) = self.lookin.open {
                    self.lookin.take();
                    if let Some(c) = self.crumbs.get(i) {
                        return Event::NeedListing(c.id);
                    }
                    return Event::Redraw;
                }
                self.filetype.take();
                return Event::Redraw;
            }
        }
        if self.focus == Focus::Extra && !matches!(key, form::Key::Tab { .. }) {
            let open = self.extra_list_open();
            match key {
                form::Key::Escape | form::Key::Enter if !open => {}
                _ => {
                    return match &mut self.extra {
                        Some((f, _)) => match f.key(key) {
                            form::Event::None => Event::None,
                            _ => Event::Redraw,
                        },
                        None => Event::None,
                    };
                }
            }
        }
        if matches!(key, form::Key::Escape) {
            return Event::Cancel;
        }
        if let form::Key::Tab { back } = key {
            self.advance_focus(back);
            return Event::Redraw;
        }
        // Space presses a focused push button as Enter does; hosts send
        // it as typed text, which the File name field keeps.
        let space = match &key {
            form::Key::Space => true,
            form::Key::Edit(EditKey::Insert(s)) => s == " ",
            _ => false,
        };
        let pressed = matches!(key, form::Key::Enter)
            || (space && matches!(self.focus, Focus::Accept | Focus::Cancel));
        if pressed && self.focus != Focus::Cancel {
            return self.activate(false);
        }
        if pressed && self.focus == Focus::Cancel {
            return Event::Cancel;
        }
        // Backspace edits the File name field; anywhere else it goes up
        // a folder.
        if self.focus != Focus::FileName
            && matches!(
                key,
                form::Key::Backspace | form::Key::Edit(EditKey::Backspace { .. })
            )
        {
            return self.up();
        }
        match self.focus {
            Focus::FileName => match key {
                form::Key::Edit(ek) => {
                    if self.filename.key(ek) {
                        self.typed = self.filename.text.clone();
                        Event::Redraw
                    } else {
                        Event::None
                    }
                }
                _ => Event::None,
            },
            Focus::FileType => combo_key(&mut self.filetype, key),
            Focus::LookIn => combo_key(&mut self.lookin, key),
            Focus::Accept | Focus::Cancel | Focus::Extra => Event::None,
            Focus::Places => {
                // The nearest live tile past the selection: a greyed one
                // refuses the keyboard as it refuses a click.
                let live = |i: &usize| self.places.places[*i].enabled;
                let next = match (key, self.places.selected) {
                    (form::Key::Up, Some(i)) => (0..i).rev().find(live),
                    (form::Key::Down, sel) => {
                        (sel.map_or(0, |i| i + 1)..self.places.places.len()).find(live)
                    }
                    _ => None,
                };
                match next {
                    Some(i) => {
                        self.places.selected = Some(i);
                        Event::Redraw
                    }
                    None => Event::None,
                }
            }
            Focus::Shell => {
                let lay = self.shell_layout(r, cm);
                self.shell_key(key, ch, &lay, mods)
            }
        }
    }

    fn shell_key(
        &mut self,
        key: form::Key,
        ch: Option<char>,
        lay: &shellview::Layout,
        mods: Mods,
    ) -> Event {
        if matches!(key, form::Key::Space) && self.multi() && mods.ctrl {
            if let Some(i) = self.shell.selection.focus {
                self.shell.selection.click(
                    i,
                    Mods {
                        ctrl: true,
                        shift: false,
                    },
                );
                self.sync_name_field();
                return Event::Redraw;
            }
            return Event::None;
        }
        if let Some(c) = ch.filter(|c| *c != ' ')
            && shellview::type_ahead(&mut self.shell, c)
        {
            return Event::Redraw;
        }
        // Hosts send Left, Right, Home and End as edit keys, which only the File name field edits with.
        let key = match key {
            form::Key::Edit(ek) => form::flat_key(&ek).unwrap_or(form::Key::Edit(ek)),
            k => k,
        };
        let dir = match key {
            form::Key::Up => IconMove::Up,
            form::Key::Down => IconMove::Down,
            form::Key::Left => IconMove::Left,
            form::Key::Right => IconMove::Right,
            form::Key::Home => IconMove::Home,
            form::Key::End => IconMove::End,
            _ => return Event::None,
        };
        let mods = self.sel_mods(mods);
        if shellview::step(&mut self.shell, dir, lay, mods) {
            shellview::follow(&mut self.shell, lay);
            if self.multi() {
                self.sync_name_field();
            }
            Event::Redraw
        } else {
            Event::None
        }
    }

    pub fn hit(&self, text: &TextRenderer, px: u32, r: Rect, x: i32, y: i32) -> Option<Hit> {
        let lay_cm = self.drawn.0.get();
        let lay = self.layout(text, px, r, &lay_cm);
        if let Some((p, menu)) = &lay.views {
            return menu.hit(x - p.x, y - p.y).map(Hit::ViewsItem);
        }
        // An open Files of type list is drawn over the extra rows, so it takes
        // the click first unless an extra combo has its list open.
        if self.filetype.open.is_some()
            && !self.extra_list_open()
            && let Some(h) = combo::hit(&self.filetype, &lay.filetype, x, y)
        {
            return Some(Hit::FileType(h));
        }
        if let Some((f, _)) = &self.extra
            && let Some(h) = f.hit(x - r.x, y - rows(r, &lay_cm).y - self.grow(r), text, px)
        {
            return Some(Hit::Extra(h));
        }
        if let Some(i) = toolbar::hit(&self.toolbar, &lay.toolbar_rects, x, y) {
            return Some(Hit::ToolBar(i));
        }
        if self.filetype.open.is_some()
            && let Some(h) = combo::hit(&self.filetype, &lay.filetype, x, y)
        {
            return Some(Hit::FileType(h));
        }
        if lay.filename.contains(x, y) {
            return Some(Hit::FileName);
        }
        if self.has_types()
            && let Some(h) = combo::hit(&self.filetype, &lay.filetype, x, y)
        {
            return Some(Hit::FileType(h));
        }
        if lay.accept.contains(x, y) {
            return Some(Hit::Accept);
        }
        if lay.cancel.contains(x, y) {
            return Some(Hit::Cancel);
        }
        // Cancel's right end overlaps the grip's square.
        if lay.grip.contains(x, y) {
            return Some(Hit::Grip);
        }
        if let Some(i) = placesbar::hit(&lay.places_rects, x, y) {
            return Some(Hit::Places(i));
        }
        // A dropped Look in list overhangs the shell view, whose `hit` answers
        // `Background` anywhere in its frame, so test the list first.
        if self.lookin.open.is_some()
            && let Some(h) = combo::hit(&self.lookin, &lay.lookin, x, y)
        {
            return Some(Hit::LookIn(h));
        }
        let h = shellview::hit(&self.shell, &lay.shell, x, y);
        if !matches!(h, shellview::Hit::Nothing) {
            return Some(Hit::Shell(h));
        }
        if let Some(h) = combo::hit(&self.lookin, &lay.lookin, x, y) {
            return Some(Hit::LookIn(h));
        }
        None
    }

    /// Dispatches a hit from `hit` into the sub-control it named and
    /// returns the resulting `Event`. `r` is what `hit` was given.
    pub fn click(&mut self, r: Rect, hit: Option<Hit>, mods: Mods) -> Event {
        let before = self.filetype.selected;
        let cm = self.drawn.0.get();
        let ev = self.click_inner(r, hit, mods, &cm);
        self.filter_event(before, ev)
    }

    fn click_inner(&mut self, r: Rect, hit: Option<Hit>, mods: Mods, cm: &ControlMetrics) -> Event {
        if self.views_open {
            self.views_open = false;
            return match hit {
                Some(Hit::ViewsItem(i)) => {
                    if let Some((_, mode)) = views_items().get(i) {
                        self.shell.set_mode(*mode);
                    }
                    Event::Redraw
                }
                _ => Event::Redraw,
            };
        }
        match hit {
            Some(Hit::ToolBar(i)) => self.toolbar_click(i),
            Some(Hit::FileName) => {
                self.focus = Focus::FileName;
                Event::Redraw
            }
            Some(Hit::FileType(h)) => {
                self.focus = Focus::FileType;
                combo_click(&mut self.filetype, h);
                Event::Redraw
            }
            Some(Hit::LookIn(h)) => self.lookin_click(h),
            Some(Hit::Accept) => {
                self.focus = Focus::Accept;
                self.activate(true)
            }
            Some(Hit::Cancel) => Event::Cancel,
            Some(Hit::Grip) => Event::Resize,
            Some(Hit::Places(i)) => {
                if !self.places.places.get(i).is_some_and(|p| p.enabled) {
                    return Event::None;
                }
                self.focus = Focus::Places;
                self.places.selected = Some(i);
                Event::NeedPlace(i)
            }
            Some(Hit::Shell(h)) => {
                if let shellview::Hit::Header(i) = h {
                    self.shell.header.press(Some(i), false);
                    shellview::sort_by_column(&mut self.shell, i);
                }
                let lay = self.shell_layout(r, cm);
                self.shell_click(h, &lay, mods)
            }
            Some(Hit::ViewsItem(_)) => Event::None,
            Some(Hit::Extra(h)) => {
                self.focus = Focus::Extra;
                let Some((f, _)) = &mut self.extra else {
                    return Event::None;
                };
                let a = f.click(Some(h));
                let b = f.release();
                if matches!((a, b), (form::Event::None, form::Event::None)) {
                    Event::None
                } else {
                    Event::Redraw
                }
            }
            None => Event::None,
        }
    }

    fn toolbar_click(&mut self, i: usize) -> Event {
        match i {
            TOOLBAR_BACK => self.nav(Nav::Back),
            TOOLBAR_UP => self.up(),
            TOOLBAR_NEW_FOLDER => Event::CreateFolder("New Folder".into()),
            TOOLBAR_VIEWS => {
                self.views_open = true;
                Event::Redraw
            }
            _ => Event::None,
        }
    }

    fn lookin_click(&mut self, h: combo::Hit) -> Event {
        self.focus = Focus::LookIn;
        if let combo::Hit::Row(i) = h {
            combo_click(&mut self.lookin, h);
            if let Some(c) = self.crumbs.get(i) {
                return Event::NeedListing(c.id);
            }
        } else {
            combo_click(&mut self.lookin, h);
        }
        Event::Redraw
    }

    fn shell_click(&mut self, h: shellview::Hit, lay: &shellview::Layout, mods: Mods) -> Event {
        self.focus = Focus::Shell;
        match h {
            // A hit taken before `set_listing` shrank the folder names no
            // entry, and lands as one on the background.
            shellview::Hit::Entry(i) => {
                // Whatever is selected goes into File name, directory or not:
                // `activate` prefers the field, so a stale name there would
                // accept that file instead of entering the folder.
                if i < self.shell.entries.len() {
                    let mods = self.sel_mods(mods);
                    self.shell.selection.click(i, mods);
                    self.sync_name_field();
                }
            }
            // Both bars' arrows and tracks. The thumbs are not dragged
            // here: the dialog tracks no press but the toolbar's.
            shellview::Hit::VBar(BarHit::Back)
            | shellview::Hit::VBar(BarHit::Forward)
            | shellview::Hit::VBar(BarHit::PageBack)
            | shellview::Hit::VBar(BarHit::PageForward)
            | shellview::Hit::HBar(BarHit::Back)
            | shellview::Hit::HBar(BarHit::Forward)
            | shellview::Hit::HBar(BarHit::PageBack)
            | shellview::Hit::HBar(BarHit::PageForward) => {
                let sv = &mut self.shell;
                match h {
                    shellview::Hit::VBar(BarHit::Back) => shellview::scroll(sv, -1, lay),
                    shellview::Hit::VBar(BarHit::Forward) => shellview::scroll(sv, 1, lay),
                    shellview::Hit::VBar(BarHit::PageBack) => shellview::page(sv, -1, lay),
                    shellview::Hit::VBar(BarHit::PageForward) => shellview::page(sv, 1, lay),
                    shellview::Hit::HBar(BarHit::Back) => shellview::hstep(sv, -1, lay),
                    shellview::Hit::HBar(BarHit::Forward) => shellview::hstep(sv, 1, lay),
                    shellview::Hit::HBar(BarHit::PageBack) => shellview::hpage(sv, -1, lay),
                    _ => shellview::hpage(sv, 1, lay),
                };
            }
            _ => {}
        }
        Event::Redraw
    }

    /// A double click on an entry enters a folder, or accepts a file
    /// (outside folder mode); anywhere else it is one more click.
    pub fn double_click(&mut self, r: Rect, hit: Option<Hit>) -> Event {
        let Some(Hit::Shell(shellview::Hit::Entry(i))) = hit else {
            return self.click(r, hit, Mods::default());
        };
        let Some(entry) = self.shell.entries.get(i) else {
            return Event::None;
        };
        let (id, name, is_dir) = (entry.id, entry.name.clone(), entry.is_dir);
        self.focus = Focus::Shell;
        self.shell.selection.set_only(Some(i));
        if is_dir {
            self.entering(&name);
            return Event::NeedListing(id);
        }
        if self.fields.mode == DlgMode::Folder {
            return Event::Redraw;
        }
        Event::Accept(vec![name])
    }

    /// The wheel turned `rows` rows (down is positive) over `hit`. An open
    /// list scrolls wherever the pointer is (extra rows' lists go to their
    /// form); otherwise the shell view scrolls, and a focused closed Look in
    /// or Files of type under the pointer steps per notch like Down and Up.
    pub fn wheel(&mut self, r: Rect, hit: Option<Hit>, rows: i32) -> Event {
        let before = self.filetype.selected;
        let ev = self.wheel_inner(r, hit, rows);
        self.filter_event(before, ev)
    }

    fn wheel_inner(&mut self, r: Rect, hit: Option<Hit>, rows: i32) -> Event {
        let redraw = |moved: bool| if moved { Event::Redraw } else { Event::None };
        if self.views_open {
            return Event::None;
        }
        for c in [&mut self.lookin, &mut self.filetype] {
            if c.open.is_some() {
                return redraw(c.scroll(rows));
            }
        }
        if self.extra_list_open() || matches!(hit, Some(Hit::Extra(_))) {
            let h = match hit {
                Some(Hit::Extra(h)) => Some(h),
                _ => None,
            };
            return match &mut self.extra {
                Some((f, _)) => redraw(!matches!(f.wheel(h, rows), form::Event::None)),
                None => Event::None,
            };
        }
        let notches = rows.signum() * (rows.abs() / WHEEL_ROWS).max(1);
        let step = |c: &mut Combo| {
            let mut changed = false;
            for _ in 0..notches.abs() {
                changed |= c.step(notches > 0);
            }
            changed
        };
        match hit {
            Some(Hit::Shell(_)) => {
                let cm = self.drawn.0.get();
                let lay = self.shell_layout(r, &cm);
                redraw(shellview::scroll(&mut self.shell, rows, &lay))
            }
            Some(Hit::FileType(_)) if self.focus == Focus::FileType => {
                redraw(step(&mut self.filetype))
            }
            Some(Hit::LookIn(_)) if self.focus == Focus::LookIn => {
                if !step(&mut self.lookin) {
                    return Event::None;
                }
                match self.crumbs.get(self.lookin.selected) {
                    Some(c) => Event::NeedListing(c.id),
                    None => Event::Redraw,
                }
            }
            _ => Event::None,
        }
    }

    /// Ends a press on a toolbar button or a Details column title, the only
    /// presses the dialog tracks.
    pub fn release(&mut self) -> Event {
        let column = self.shell.header.holding();
        self.shell.header.release();
        if self.toolbar.pressed.take().is_some() || (column && self.drawn.1.get()) {
            Event::Redraw
        } else {
            Event::None
        }
    }

    /// The pointer moved to (`x`, `y`) in `r`: the Details column title
    /// under it lights, or the held one follows it. A redraw only when
    /// that changed and the look draws it lit.
    pub fn motion(&mut self, text: &TextRenderer, px: u32, r: Rect, x: i32, y: i32) -> Event {
        let hit = self.hit(text, px, r, x, y);
        self.hover(hit)
    }

    /// [`FileDlg::motion`] for a host that hit-tested the point already.
    /// An open combo's highlight follows the pointer down its list.
    pub fn hover(&mut self, hit: Option<Hit>) -> Event {
        let followed = match hit {
            Some(Hit::LookIn(combo::Hit::Row(i))) => self.lookin.hover(i),
            Some(Hit::FileType(combo::Hit::Row(i))) => self.filetype.hover(i),
            Some(Hit::Extra(form::Hit {
                item,
                part: form::Part::Combo(combo::Hit::Row(i)),
                ..
            })) => match self.extra.as_mut().and_then(|(f, _)| f.items.get_mut(item)) {
                Some(form::Item {
                    control: form::Control::Combo(c),
                    ..
                }) => c.hover(i),
                _ => false,
            },
            _ => false,
        };
        if followed {
            return Event::Redraw;
        }
        let column = match hit {
            Some(Hit::Shell(shellview::Hit::Header(i))) => Some(i),
            _ => None,
        };
        let changed = self.shell.header.motion(column);
        self.lit(changed)
    }

    pub fn leave(&mut self) -> Event {
        let changed = self.shell.header.leave();
        self.lit(changed)
    }

    fn lit(&self, changed: bool) -> Event {
        if changed && self.drawn.1.get() {
            Event::Redraw
        } else {
            Event::None
        }
    }

    /// Draws the client area into `r`; the title bar is the host's chrome.
    pub fn draw(&self, c: &mut Canvas, text: &TextRenderer, p: &Paint, r: Rect, px: u32) {
        let Paint { cs, s, .. } = *p;
        self.drawn.0.set(p.look.controls().grown(p.text));
        self.drawn.1.set(p.look.lights_pointer());
        let lay = self.layout(text, px, r, &self.drawn.0.get());
        c.fill_rect(r.scaled(s), cs.button_face);

        text.draw(
            c,
            lay.lookin_label.scaled(s),
            "Look in:",
            false,
            cs.button_text,
            Align::Left,
            px,
        );
        combo::draw(
            c,
            text,
            p,
            &self.lookin,
            &lay.lookin,
            self.focus == Focus::LookIn,
            true,
            Caret::Unfocused,
            combo::ComboLit::IDLE,
            px,
        );
        if let Some(icon) = self.crumb_icon() {
            c.blit_over(
                (lay.lookin.field.x + LOOKIN_ICON_X) * s,
                (lay.lookin.field.y + LOOKIN_ICON_Y + (lay.lookin.field.height - LOOKIN_H) / 2) * s,
                icon,
            );
        }

        toolbar::draw(
            c,
            text,
            p,
            &self.toolbar,
            lay.toolbar,
            &lay.toolbar_rects,
            px,
        );

        placesbar::draw(c, text, p, &self.places, lay.places, px);

        shellview::draw(c, text, p, &self.shell, &lay.shell, None, px);

        text.draw(
            c,
            lay.filename_label.scaled(s),
            "File name:",
            false,
            cs.button_text,
            Align::Left,
            px,
        );
        // Its frame shows the focus; the field tracks no pointer.
        let focused = self.focus == Focus::FileName;
        let inner = draw_edit_field(
            c,
            p,
            lay.filename.scaled(s),
            cs.window,
            focused,
            Interaction::Idle,
        );
        let first = match self.name_drop {
            Some((_, first)) => first.min(self.filename.text.len()),
            None => edit_first_visible(text, &self.filename, inner.width, px, 0),
        };
        draw_edit(
            c,
            text,
            &self.filename,
            inner,
            px,
            p,
            Caret::new(self.focus == Focus::FileName, self.name_drop.is_none()),
            first,
        );
        if let Some((x, _)) = self.name_drop {
            let tail = &self.filename.text[first..];
            let at = first + crate::textedit::index_at(text, tail, x * s - inner.x, px);
            widgets::draw_edit_drop_point(c, text, &self.filename, inner, px, p, first, at);
        }
        draw_button(
            c,
            text,
            p,
            lay.accept.scaled(s),
            &self.fields.accept_label,
            true,
            Interaction::Idle,
            true,
            self.focus == Focus::Accept,
            px,
        );

        if self.has_types() {
            text.draw(
                c,
                lay.filetype_label.scaled(s),
                "Files of type:",
                false,
                cs.button_text,
                Align::Left,
                px,
            );
            combo::draw(
                c,
                text,
                p,
                &self.filetype,
                &lay.filetype,
                self.focus == Focus::FileType,
                true,
                Caret::Unfocused,
                combo::ComboLit::IDLE,
                px,
            );
        }
        draw_button(
            c,
            text,
            p,
            lay.cancel.scaled(s),
            "Cancel",
            false,
            Interaction::Idle,
            true,
            self.focus == Focus::Cancel,
            px,
        );

        if let Some((f, _)) = &self.extra {
            let rows = rows(r, &self.drawn.0.get());
            let dy = self.grow(r);
            let mut shifted = f.clone();
            shifted.origin = (rows.x, rows.y + dy);
            if self.focus != Focus::Extra {
                shifted.set_focus(None);
            }
            form::draw(c, text, p, &shifted, true, px);
            f.drawn_with(p);
        }
        if self.lookin.open.is_some() {
            combo::draw_list(
                c,
                text,
                p,
                &self.lookin.items,
                self.lookin.open,
                self.lookin.top,
                self.lookin.rows,
                lay.lookin.list,
                lay.lookin.scrollbar,
                None,
                px,
            );
        }
        if self.filetype.open.is_some() {
            combo::draw_list(
                c,
                text,
                p,
                &self.filetype.items,
                self.filetype.open,
                self.filetype.top,
                self.filetype.rows,
                lay.filetype.list,
                lay.filetype.scrollbar,
                None,
                px,
            );
        }

        crate::look::widgets::draw_size_grip(c, p, lay.grip.scaled(s));

        if let Some((at, menu)) = &lay.views {
            crate::controls::menu::draw_menu_at(
                c,
                (at.x * s, at.y * s),
                &Theme::for_draw(p, px),
                text,
                &menu.scaled(s),
                &self.views_menu_items(),
                None,
                "",
                false,
            );
        }
    }
}

impl FileDlg {
    /// The dialog's `access` part, laid out in `r` like [`FileDlg::draw`].
    pub fn access(&self, tree: &mut Tree, r: Rect, text: &TextRenderer, px: u32) {
        let root = access::ROOT;
        let lay = self.layout(text, px, r, &self.drawn.0.get());
        let label = |tree: &mut Tree, key: &str, rect: Rect, s: &str| {
            tree.push(root, key, access::label(rect, s, true))
        };
        let labelled = |tree: &mut Tree, id: NodeId, by: NodeId| {
            if let Some(n) = tree.get_mut(id) {
                n.push_labelled_by(by);
            }
        };
        let l = label(tree, "lookin label", lay.lookin_label, "Look in:");
        let lookin = combo::access(&self.lookin, &lay.lookin, text, px, tree, root, "lookin");
        labelled(tree, Tree::id_of(root, "lookin"), l);
        toolbar::access(
            &self.toolbar,
            lay.toolbar,
            &lay.toolbar_rects,
            tree,
            root,
            "toolbar",
        );
        let places = placesbar::access(
            &self.places,
            lay.places,
            &self.drawn.0.get().places,
            tree,
            root,
            "places",
        );
        let shell = shellview::access(&self.shell, &lay.shell, text, px, tree, root, "shell");
        let l = label(tree, "filename label", lay.filename_label, "File name:");
        let first = edit_first_visible(
            text,
            &self.filename,
            lay.filename.inset(widgets::FIELD_PAD).width,
            px,
            0,
        );
        let filename = widgets::access_edit(
            &self.filename,
            first,
            false,
            lay.filename,
            true,
            text,
            px,
            tree,
            root,
            "filename",
        );
        labelled(tree, filename, l);
        let accept = tree.push(
            root,
            "accept",
            access::button(lay.accept, &self.fields.accept_label, true, true),
        );
        let filetype = self.has_types().then(|| {
            let l = label(tree, "filetype label", lay.filetype_label, "Files of type:");
            let filetype = combo::access(
                &self.filetype,
                &lay.filetype,
                text,
                px,
                tree,
                root,
                "filetype",
            );
            labelled(tree, Tree::id_of(root, "filetype"), l);
            filetype
        });
        let cancel = tree.push(
            root,
            "cancel",
            access::button(lay.cancel, "Cancel", true, false),
        );
        let extra = self
            .extra
            .as_ref()
            .and_then(|(f, _)| f.access(tree, root, text, px));
        tree.set_focus(match self.focus {
            Focus::FileName => filename,
            Focus::FileType => filetype.unwrap_or(filename),
            Focus::Extra => extra.or(filetype).unwrap_or(filename),
            Focus::Accept => accept,
            Focus::Cancel => cancel,
            Focus::Places => places,
            Focus::Shell => shell,
            Focus::LookIn => lookin,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::classic_paint;
    use crate::theme::ColorScheme;

    const R: Rect = Rect::new(0, 0, CLIENT.0, CLIENT.1);

    fn text() -> TextRenderer {
        TextRenderer::load(&crate::theme::FontConfig::default())
    }

    fn sample() -> FileDlg {
        FileDlg::new(
            Fields {
                title: "Open".into(),
                accept_label: "Open".into(),
                filters: vec!["All Files (*.*)".into()],
                mode: DlgMode::Open,
                multiple: false,
            },
            vec![Place {
                label: "My Computer".into(),
                icon: None,
                enabled: true,
            }],
        )
    }

    #[test]
    fn a_type_label_carries_its_pattern_in_parentheses() {
        assert_eq!(type_pattern("Text Documents (*.txt)"), Some("*.txt"));
        assert_eq!(type_pattern("All Files (*.*)"), Some("*.*"));
        assert_eq!(
            type_pattern("Background Files (*.bmp;*.png) "),
            Some("*.bmp;*.png")
        );
        assert_eq!(type_pattern("Programs"), None);
        assert_eq!(
            type_pattern("Notes (draft)"),
            None,
            "no wildcard, no pattern"
        );
    }

    #[test]
    fn a_pattern_keeps_names_any_of_its_globs_match() {
        assert!(pattern_keeps("*.txt", "notes.TXT"));
        assert!(!pattern_keeps("*.txt", "bash"));
        assert!(pattern_keeps("*.txt; *.log", "boot.log"));
        assert!(
            pattern_keeps("*.*", "bash"),
            "*.* keeps a name without a dot"
        );
        assert!(pattern_keeps("*", "bash"));
        assert!(pattern_keeps("b?sh", "bash"));
        assert!(!pattern_keeps("", "bash"));
    }

    #[test]
    fn the_picked_type_gives_its_pattern() {
        let mut d = FileDlg::new(
            Fields {
                title: "Open".into(),
                accept_label: "Open".into(),
                filters: vec!["Text Documents (*.txt)".into(), "All Files (*.*)".into()],
                mode: DlgMode::Open,
                multiple: false,
            },
            vec![],
        );
        assert_eq!(d.picked_pattern().as_deref(), Some("*.txt"));
        d.set_filter(1);
        assert_eq!(d.picked_pattern().as_deref(), Some("*.*"));
    }

    /// The Views popup is 91 px tall (five 17 px rows and the 3 px border).
    #[test]
    fn views_popup_is_a_context_menu_under_its_button() {
        let text = text();
        let mut d = sample();
        let r = R;
        assert!(matches!(
            d.click(r, Some(Hit::ToolBar(TOOLBAR_VIEWS)), Mods::default()),
            Event::Redraw
        ));
        assert!(d.views_open);
        let lay = d.layout(&text, 11, r, &ControlMetrics::CLASSIC);
        let (p, menu) = lay
            .views
            .as_ref()
            .expect("the popup is laid out while open");
        let p = *p;
        assert_eq!(p.height, 5 * 17 + 6);
        assert_eq!(p.y, lay.toolbar_rects[TOOLBAR_VIEWS].bottom());
        assert_eq!(p.x, lay.toolbar_rects[TOOLBAR_VIEWS].x);
        let third = menu.rows[2];
        let (x, y) = (
            p.x + third.x + third.width / 2,
            p.y + third.y + third.height / 2,
        );
        assert_eq!(d.hit(&text, 11, r, x, y), Some(Hit::ViewsItem(2)));
        assert_eq!(
            d.hit(&text, 11, r, p.x + 1, p.y + 1),
            None,
            "the border is not a row"
        );
        assert_eq!(d.hit(&text, 11, r, p.right() + 5, y), None);
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        d.draw(
            &mut c,
            &text,
            &classic_paint(&ColorScheme::default(), 1),
            r,
            11,
        );
        assert!(matches!(
            d.click(r, Some(Hit::ViewsItem(4)), Mods::default()),
            Event::Redraw
        ));
        assert_eq!(d.shell.mode, Mode::Details);
        assert!(!d.views_open);
    }

    /// The toolbar's four buttons run from its left edge and end 84 px
    /// short of the client's right edge; the Views popup is laid out only
    /// while it is open.
    #[test]
    fn toolbar_ends_84_px_short_of_the_right_edge() {
        let lay = multi(DlgMode::Open, false).layout(&text(), 11, R, &ControlMetrics::CLASSIC);
        assert_eq!(lay.toolbar_rects[0].x, lay.toolbar.x);
        assert_eq!(lay.toolbar_rects[TOOLBAR_VIEWS].right(), CLIENT.0 - 84);
        assert_eq!(lay.toolbar.right(), CLIENT.0 - 84);
        assert!(lay.views.is_none(), "no popup while it is closed");
    }

    /// `top` is px in Icons and rows in Details, so a Views pick starts
    /// the new mode scrolled to the top-left.
    #[test]
    fn views_pick_resets_the_scroll() {
        let mut d = sample();
        d.shell.mode = Mode::Icons;
        d.shell.top = 400;
        d.shell.left = 30;
        d.click(R, Some(Hit::ToolBar(TOOLBAR_VIEWS)), Mods::default());
        d.click(R, Some(Hit::ViewsItem(4)), Mods::default());
        assert_eq!(d.shell.mode, Mode::Details);
        assert_eq!((d.shell.top, d.shell.left), (0, 0), "top and left are 0");
    }

    fn folder(id: FolderId, parent: Option<FolderId>, entries: Vec<shellview::Entry>) -> Folder {
        Folder {
            id,
            parent,
            crumbs: vec![Crumb {
                id,
                label: "Folder".into(),
                depth: 0,
                icon: None,
            }],
            entries,
        }
    }

    fn dir(name: &str) -> shellview::Entry {
        shellview::Entry {
            id: 900,
            name: name.into(),
            is_dir: true,
            kind: "File Folder".into(),
            ..Default::default()
        }
    }

    fn file(name: &str) -> shellview::Entry {
        shellview::Entry {
            name: name.into(),
            kind: "Text Document".into(),
            ..Default::default()
        }
    }

    fn multi(mode: DlgMode, multiple: bool) -> FileDlg {
        let mut d = FileDlg::new(
            Fields {
                title: "Open".into(),
                accept_label: "Open".into(),
                filters: vec!["All Files".into(), "Text".into()],
                mode,
                multiple,
            },
            vec![],
        );
        d.set_listing(folder(
            1,
            None,
            vec![dir("sub"), file("a.txt"), file("b.txt"), file("c.txt")],
        ));
        d
    }

    const CTRL: Mods = Mods {
        ctrl: true,
        shift: false,
    };
    const SHIFT: Mods = Mods {
        ctrl: false,
        shift: true,
    };

    fn entry_hit(i: usize) -> Option<Hit> {
        Some(Hit::Shell(shellview::Hit::Entry(i)))
    }

    #[test]
    fn ctrl_and_shift_clicks_select_several_files_when_multiple() {
        let mut d = multi(DlgMode::Open, true);
        d.click(R, entry_hit(1), Mods::default());
        d.click(R, entry_hit(3), CTRL);
        assert_eq!(d.file_name(), "\"c.txt\" \"a.txt\"");
        d.click(R, entry_hit(2), SHIFT);
        assert_eq!(
            d.file_name(),
            "\"b.txt\" \"a.txt\"",
            "shift ranges from the anchor"
        );
    }

    #[test]
    fn several_names_start_at_the_focused_file() {
        let mut d = multi(DlgMode::Open, true);
        d.click(R, entry_hit(3), Mods::default());
        d.click(R, entry_hit(1), CTRL);
        d.click(R, entry_hit(2), CTRL);
        assert_eq!(d.file_name(), "\"b.txt\" \"c.txt\" \"a.txt\"");
        d.click(R, entry_hit(1), CTRL);
        assert_eq!(
            d.file_name(),
            "\"b.txt\" \"c.txt\"",
            "a file clicked out of the pick starts the walk"
        );
    }

    #[test]
    fn folder_mode_has_no_files_of_type_row() {
        let mut d = multi(DlgMode::Folder, false);
        let t = text();
        let theme = Theme::default();
        let cs = &theme.colors;
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        c.fill_rect(c.bounds(), cs.button_face);
        d.draw(&mut c, &t, &classic_paint(cs, 1), R, 11);
        for y in FILETYPE_Y..FILETYPE_Y + FILETYPE_H {
            for x in ROW_LABEL_X..FIELD_X + field_w(CLIENT.0) {
                assert_eq!(
                    c.get(x, y),
                    cs.button_face.argb(),
                    "({x}, {y}) is bare dialog face"
                );
            }
        }
        assert!(
            !matches!(
                d.hit(&t, 11, R, FIELD_X + 20, FILETYPE_Y + 10),
                Some(Hit::FileType(_))
            ),
            "no Files of type combo to hit"
        );
        d.focus = Focus::FileName;
        d.key(R, form::Key::Tab { back: false }, None, Mods::default());
        assert_eq!(d.focus, Focus::Accept, "Tab goes from File name to Select");
        d.key(R, form::Key::Tab { back: true }, None, Mods::default());
        assert_eq!(d.focus, Focus::FileName, "and back");
    }

    #[test]
    fn without_multiple_ctrl_click_replaces_the_selection() {
        let mut d = multi(DlgMode::Open, false);
        d.click(R, entry_hit(1), Mods::default());
        d.click(R, entry_hit(3), CTRL);
        assert_eq!(d.file_name(), "c.txt");
    }

    #[test]
    fn a_folder_in_a_multi_selection_stays_out_of_the_field() {
        let mut d = multi(DlgMode::Open, true);
        d.click(R, entry_hit(0), Mods::default());
        d.click(R, entry_hit(1), CTRL);
        d.click(R, entry_hit(2), CTRL);
        assert_eq!(d.file_name(), "\"b.txt\" \"a.txt\"");
    }

    #[test]
    fn keyboard_multi_selection() {
        let mut d = multi(DlgMode::Open, true);
        d.click(R, entry_hit(1), Mods::default());
        d.key(R, form::Key::Down, None, SHIFT);
        assert_eq!(d.file_name(), "\"b.txt\" \"a.txt\"");
        d.key(R, form::Key::Down, None, CTRL);
        d.key(R, form::Key::Space, None, CTRL);
        assert_eq!(d.file_name(), "\"c.txt\" \"a.txt\" \"b.txt\"");
    }

    #[test]
    fn accept_carries_every_selected_name() {
        let mut d = multi(DlgMode::Open, true);
        d.click(R, entry_hit(1), Mods::default());
        d.click(R, entry_hit(2), CTRL);
        match d.click(R, Some(Hit::Accept), Mods::default()) {
            Event::Accept(names) => assert_eq!(names, vec!["b.txt", "a.txt"]),
            e => panic!("expected Accept, got {e:?}"),
        }
    }

    #[test]
    fn quoted_names_split_and_a_plain_name_stays_whole() {
        assert_eq!(
            split_names("\"a b.txt\" \"c.txt\""),
            vec!["a b.txt", "c.txt"]
        );
        assert_eq!(split_names("a b.txt"), vec!["a b.txt"]);
        assert_eq!(split_names("  "), Vec::<String>::new());
        assert_eq!(
            split_names("x \"y\""),
            vec!["x \"y\""],
            "quotes split only from the start"
        );
    }

    #[test]
    fn folder_mode_accepts_the_selected_subfolder_from_the_button() {
        let mut d = multi(DlgMode::Folder, false);
        d.click(R, entry_hit(0), Mods::default());
        match d.click(R, Some(Hit::Accept), Mods::default()) {
            Event::Accept(names) => assert_eq!(names, vec!["sub"]),
            e => panic!("expected Accept, got {e:?}"),
        }
    }

    #[test]
    fn folder_mode_enter_on_a_subfolder_opens_it() {
        let mut d = multi(DlgMode::Folder, false);
        d.click(R, entry_hit(0), Mods::default());
        assert!(matches!(
            d.key(R, form::Key::Enter, None, Mods::default()),
            Event::NeedListing(_)
        ));
    }

    #[test]
    fn folder_mode_with_nothing_picked_accepts_the_open_folder() {
        let mut d = multi(DlgMode::Folder, false);
        d.shell.selection.set_only(None);
        d.filename.set_selected("");
        match d.click(R, Some(Hit::Accept), Mods::default()) {
            Event::Accept(names) => assert_eq!(names, vec!["."]),
            e => panic!("expected Accept, got {e:?}"),
        }
    }

    #[test]
    fn folder_mode_accepts_the_open_folder_right_after_listing() {
        let mut d = multi(DlgMode::Folder, false);
        assert_eq!(
            d.shell.selection.focus, None,
            "a folder listing starts with nothing selected"
        );
        match d.click(R, Some(Hit::Accept), Mods::default()) {
            Event::Accept(names) => assert_eq!(names, vec!["."]),
            e => panic!("expected Accept, got {e:?}"),
        }
    }

    #[test]
    fn the_files_of_type_list_opens_above_when_it_would_pass_the_bottom() {
        let mut d = multi(DlgMode::Open, false);
        let t = text();
        let field = d.layout(&t, 11, R, &ControlMetrics::CLASSIC).filetype.field;
        d.click(R, Some(Hit::FileType(combo::Hit::Drop)), Mods::default());
        let list = d.layout(&t, 11, R, &ControlMetrics::CLASSIC).filetype.list;
        assert!(
            list.bottom() <= R.bottom(),
            "the list ends inside the client"
        );
        assert_eq!(list.bottom(), field.y, "it sits right on top of the field");
        let hit = d.hit(&t, 11, R, list.x + 10, list.y + 3);
        assert_eq!(
            hit,
            Some(Hit::FileType(combo::Hit::Row(0))),
            "its first row takes the click"
        );
    }

    fn save_dlg() -> FileDlg {
        let mut d = multi(DlgMode::Save, false);
        d.set_file_name("report.pdf");
        d
    }

    /// A typed `*` or `?` pattern is not accepted: the host applies it, which
    /// filters the view and leaves the pattern in the field.
    #[test]
    fn a_typed_pattern_filters_instead_of_accepting() {
        let mut d = multi(DlgMode::Open, false);
        d.set_file_name("*");
        match d.key(R, form::Key::Enter, None, Mods::default()) {
            Event::Wildcard { dir, name } => assert_eq!((dir.as_str(), name.as_str()), ("", "*")),
            e => panic!("expected Wildcard, got {e:?}"),
        }
        d.apply_wildcard("*");
        assert_eq!(d.wildcard(), Some("*"));
        assert_eq!(d.file_name(), "*");
        d.set_file_name("/etc/*.conf");
        let name = match d.click(R, Some(Hit::Accept), Mods::default()) {
            Event::Wildcard { dir, name } => {
                assert_eq!((dir.as_str(), name.as_str()), ("/etc/", "/etc/*.conf"));
                name
            }
            e => panic!("expected Wildcard, got {e:?}"),
        };
        assert_eq!(d.wildcard(), Some("*"), "the old filter until applied");
        assert_eq!(d.file_name(), "/etc/*.conf", "the field as typed");
        d.apply_wildcard(&name);
        assert_eq!(d.wildcard(), Some("*.conf"));
        assert_eq!(d.file_name(), "*.conf", "the field keeps the pattern alone");
        d.set_file_name("/e*/a.txt");
        assert!(
            matches!(
                d.key(R, form::Key::Enter, None, Mods::default()),
                Event::Accept(_)
            ),
            "a pattern in the folder part is a name"
        );
    }

    #[test]
    fn picking_a_type_drops_the_typed_pattern() {
        let mut d = multi(DlgMode::Open, false);
        d.set_file_name("*.txt");
        d.apply_wildcard("*.txt");
        assert_eq!(d.wildcard(), Some("*.txt"));
        d.key(R, form::Key::Tab { back: false }, None, Mods::default());
        assert!(matches!(
            d.key(R, form::Key::Down, None, Mods::default()),
            Event::FilterChanged(1)
        ));
        assert_eq!(d.wildcard(), None);
    }

    #[test]
    fn a_listed_name_or_folder_mode_takes_no_pattern() {
        let mut d = multi(DlgMode::Open, false);
        d.set_listing(folder(2, None, vec![file("why?.txt")]));
        d.set_file_name("why?.txt");
        assert!(matches!(
            d.key(R, form::Key::Enter, None, Mods::default()),
            Event::Accept(_)
        ));
        assert_eq!(d.wildcard(), None);
        let mut d = multi(DlgMode::Folder, false);
        d.set_file_name("*");
        assert!(matches!(
            d.key(R, form::Key::Enter, None, Mods::default()),
            Event::Accept(_)
        ));
    }

    #[test]
    fn entering_a_clicked_folder_puts_the_suggested_name_back() {
        let mut d = save_dlg();
        d.click(R, entry_hit(0), Mods::default());
        assert_eq!(d.file_name(), "sub", "a click shows the folder");
        assert!(matches!(
            d.key(R, form::Key::Enter, None, Mods::default()),
            Event::NeedListing(_)
        ));
        assert_eq!(d.file_name(), "report.pdf", "the suggested name is back");
    }

    #[test]
    fn entering_a_typed_folder_name_empties_the_field() {
        let mut d = multi(DlgMode::Open, false);
        d.shell.selection.set_only(Some(0));
        d.filename.set_selected("");
        d.typed.clear();
        for c in "sub".chars() {
            d.key(
                R,
                form::Key::Edit(crate::textedit::EditKey::Insert(c.to_string())),
                None,
                Mods::default(),
            );
        }
        assert!(matches!(
            d.key(R, form::Key::Enter, None, Mods::default()),
            Event::NeedListing(_)
        ));
        assert_eq!(d.file_name(), "");
    }

    #[test]
    fn folder_mode_enter_then_select_picks_the_entered_folder() {
        let mut d = multi(DlgMode::Folder, false);
        d.click(R, entry_hit(0), Mods::default());
        assert!(matches!(
            d.key(R, form::Key::Enter, None, Mods::default()),
            Event::NeedListing(_)
        ));
        d.set_listing(folder(2, Some(1), vec![]));
        match d.click(R, Some(Hit::Accept), Mods::default()) {
            Event::Accept(names) => assert_eq!(names, vec!["."]),
            e => panic!("expected Accept(.), got {e:?}"),
        }
    }

    #[test]
    fn ctrl_click_off_leaves_the_remaining_file_in_the_field() {
        let mut d = multi(DlgMode::Open, true);
        d.click(R, entry_hit(1), Mods::default());
        d.click(R, entry_hit(3), CTRL);
        d.click(R, entry_hit(3), CTRL);
        assert_eq!(d.file_name(), "a.txt");
        match d.click(R, Some(Hit::Accept), Mods::default()) {
            Event::Accept(names) => assert_eq!(names, vec!["a.txt"]),
            e => panic!("expected Accept, got {e:?}"),
        }
    }

    #[test]
    fn a_name_with_quotes_is_one_name_outside_multi_open() {
        let mut d = save_dlg();
        d.set_file_name("The \"Best\" Recipe.html");
        d.shell.selection.set_only(None);
        match d.key(R, form::Key::Enter, None, Mods::default()) {
            Event::Accept(names) => assert_eq!(names, vec!["The \"Best\" Recipe.html"]),
            e => panic!("expected Accept, got {e:?}"),
        }
        d.set_file_name(" spaced ");
        match d.key(R, form::Key::Enter, None, Mods::default()) {
            Event::Accept(names) => assert_eq!(names, vec![" spaced "]),
            e => panic!("expected Accept, got {e:?}"),
        }
    }

    #[test]
    fn many_filters_open_a_list_that_stays_inside_the_top() {
        let filters: Vec<String> = (0..40).map(|i| format!("Type {i}")).collect();
        let mut d = FileDlg::new(
            Fields {
                title: "Open".into(),
                accept_label: "Open".into(),
                filters,
                mode: DlgMode::Open,
                multiple: false,
            },
            vec![],
        );
        d.click(R, Some(Hit::FileType(combo::Hit::Drop)), Mods::default());
        let list = d
            .layout(&text(), 11, R, &ControlMetrics::CLASSIC)
            .filetype
            .list;
        assert!(
            list.y >= 0,
            "the list starts at y {} inside the client",
            list.y
        );
        assert!(list.bottom() <= R.bottom());
    }

    #[test]
    fn an_open_filter_list_takes_clicks_over_the_choice_rows() {
        let mut d = with_extra();
        let (w, h) = d.client();
        let big = Rect::new(0, 0, w, h);
        d.click(big, Some(Hit::FileType(combo::Hit::Drop)), Mods::default());
        let list = d
            .layout(&text(), 11, big, &ControlMetrics::CLASSIC)
            .filetype
            .list;
        let row1 = list.y + 1 + d.filetype.row_h(&ControlMetrics::CLASSIC) + 2;
        assert!(row1 > EXTRA_Y, "the second row lies over the extra rows");
        let hit = d.hit(&text(), 11, big, list.x + 20, row1);
        assert_eq!(hit, Some(Hit::FileType(combo::Hit::Row(1))));
        let ev = d.motion(&text(), 11, big, list.x + 20, row1);
        assert!(matches!(ev, Event::Redraw), "got {ev:?}");
        assert_eq!(d.filetype.open, Some(1));
    }

    #[test]
    fn double_clicking_a_folder_enters_it_in_every_mode() {
        for mode in [DlgMode::Open, DlgMode::Save, DlgMode::Folder] {
            let mut d = multi(mode, false);
            d.click(R, entry_hit(0), Mods::default());
            assert!(
                matches!(d.double_click(R, entry_hit(0)), Event::NeedListing(900)),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn double_clicking_a_file_accepts_it() {
        let mut d = multi(DlgMode::Open, true);
        d.click(R, entry_hit(2), Mods::default());
        match d.double_click(R, entry_hit(2)) {
            Event::Accept(names) => assert_eq!(names, vec!["b.txt"]),
            e => panic!("expected Accept, got {e:?}"),
        }
        let mut f = multi(DlgMode::Folder, false);
        assert!(
            matches!(f.double_click(R, entry_hit(2)), Event::Redraw | Event::None),
            "no file pick in folder mode"
        );
    }

    #[test]
    fn set_filter_picks_the_row_and_clamps() {
        let mut d = multi(DlgMode::Open, false);
        d.set_filter(1);
        assert_eq!(d.filter(), 1);
        d.set_filter(9);
        assert_eq!(d.filter(), 1, "clamped to the last row");
    }

    #[test]
    fn picking_another_filter_says_so() {
        let mut d = multi(DlgMode::Open, false);
        d.focus = Focus::FileType;
        assert!(matches!(
            d.key(R, form::Key::Down, None, Mods::default()),
            Event::FilterChanged(1)
        ));
        assert!(matches!(
            d.key(R, form::Key::Down, None, Mods::default()),
            Event::Redraw | Event::None
        ));
    }

    fn with_extra() -> FileDlg {
        let mut d = multi(DlgMode::Open, false);
        let w = field_w(CLIENT.0);
        let form = form::Form::new(vec![
            form::Item::check("extra-check", FIELD_X, EXTRA_Y, "Open read-only"),
            form::Item::combo(
                "extra-combo",
                FIELD_X,
                EXTRA_Y + 20,
                w,
                &["UTF-8", "Latin-1"],
                2,
            ),
        ]);
        d.set_extra(form, EXTRA_H, &ControlMetrics::CLASSIC);
        d
    }

    const EXTRA_H: i32 = 70;

    #[test]
    fn a_tight_extra_grows_to_show_up_to_four_list_rows() {
        let w = field_w(CLIENT.0);
        let combo_of = |n: usize| {
            let names: Vec<String> = (0..n).map(|i| format!("row {i}")).collect();
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            form::Form::new(vec![form::Item::combo("c", FIELD_X, EXTRA_Y, w, &refs, n)])
        };
        for (n, shown) in [(2, 2), (3, 3), (9, 4)] {
            let mut d = multi(DlgMode::Open, false);
            d.set_extra(combo_of(n), 29, &ControlMetrics::CLASSIC);
            let it = d.extra().unwrap().item("c").unwrap();
            let form::Control::Combo(c) = &it.control else {
                panic!("combo")
            };
            assert_eq!(c.rows, shown, "{n} options show {shown} rows");
            let list = combo::layout(c, it.rect, &ControlMetrics::CLASSIC).list;
            assert_eq!(
                d.client().1,
                list.bottom(),
                "the client grows to the list's bottom"
            );
        }
    }

    #[test]
    fn extra_rows_grow_the_client_and_leave_the_rest_in_place() {
        let d = with_extra();
        assert_eq!(d.client(), (CLIENT.0, CLIENT.1 + EXTRA_H));
        let big = Rect::new(0, 0, CLIENT.0, CLIENT.1 + EXTRA_H);
        let t = text();
        let base = multi(DlgMode::Open, false).layout(&t, 11, R, &ControlMetrics::CLASSIC);
        let grown = d.layout(&t, 11, big, &ControlMetrics::CLASSIC);
        assert_eq!(grown.filename, base.filename, "File name field unchanged");
        assert_eq!(grown.accept, base.accept, "accept button unchanged");
        assert_eq!(grown.cancel, base.cancel, "Cancel unchanged");
        assert_eq!(grown.shell.frame, base.shell.frame, "shell view unchanged");
    }

    #[test]
    fn a_click_on_an_extra_row_reaches_it() {
        let mut d = with_extra();
        let big = Rect::new(0, 0, CLIENT.0, CLIENT.1 + EXTRA_H);
        let hit = d.hit(&text(), 11, big, FIELD_X + 5, EXTRA_Y + 5);
        assert!(matches!(hit, Some(Hit::Extra(_))), "got {hit:?}");
        d.click(big, hit, Mods::default());
        assert_eq!(d.extra().unwrap().checked("extra-check"), Some(true));
    }

    #[test]
    fn set_size_stops_at_the_opening_size() {
        let mut d = with_extra();
        let min = d.min_client();
        assert_eq!(d.client(), min);
        d.set_size((100, 100));
        assert_eq!(d.client(), min, "no smaller than it opened");
        d.set_size((min.0 + 168, min.1 + 55));
        assert_eq!(d.client(), (min.0 + 168, min.1 + 55));
    }

    /// Grown by (168, 55): the places bar takes the height, the shell view
    /// both, Look in and the fields the width; the toolbar and buttons keep
    /// to the right edge, the rows under the shell view to the bottom.
    #[test]
    fn a_grown_dialog_stretches_the_view_and_keeps_its_rows_to_the_corner() {
        let (dw, dh) = (168, 55);
        let t = text();
        let cm = ControlMetrics::CLASSIC;
        let mut d = with_extra();
        let (w, h) = d.client();
        let base = d.layout(&t, 11, Rect::new(0, 0, w, h), &cm);
        d.set_size((w + dw, h + dh));
        let big = Rect::new(0, 0, w + dw, h + dh);
        let lay = d.layout(&t, 11, big, &cm);
        let moved = |r: Rect, x: i32, y: i32| Rect::new(r.x + x, r.y + y, r.width, r.height);
        let grown = |r: Rect, x: i32, y: i32| Rect::new(r.x, r.y, r.width + x, r.height + y);
        assert_eq!(lay.places, grown(base.places, 0, dh), "places bar");
        assert_eq!(
            lay.shell.frame,
            grown(base.shell.frame, dw, dh),
            "shell view"
        );
        assert_eq!(lay.lookin.field, grown(base.lookin.field, dw, 0), "Look in");
        assert_eq!(lay.toolbar, moved(base.toolbar, dw, 0), "toolbar");
        assert_eq!(
            lay.filename,
            moved(grown(base.filename, dw, 0), 0, dh),
            "File name"
        );
        assert_eq!(
            lay.filetype.field,
            moved(grown(base.filetype.field, dw, 0), 0, dh),
            "Files of type"
        );
        assert_eq!(lay.accept, moved(base.accept, dw, dh), "accept button");
        assert_eq!(lay.cancel, moved(base.cancel, dw, dh), "Cancel");
        let hit = d.hit(&t, 11, big, FIELD_X + 5, EXTRA_Y + dh + 5);
        assert!(matches!(hit, Some(Hit::Extra(_))), "got {hit:?}");
    }

    /// At 200% text the client opens at twice its 100% minimum and every row, field and button
    /// grows from the client's corner; the toolbar's buttons keep their px, centred on the Look in
    /// row; a host's extra rows draw and hit at their grown place.
    #[test]
    fn a_grown_text_size_grows_the_dialog() {
        let t = text();
        let u = TextUnits::new(22, 11);
        let mut d = with_extra();
        d.set_text(u);
        assert_eq!(d.min_client(), (2 * CLIENT.0, 2 * (CLIENT.1 + EXTRA_H)));
        assert_eq!(d.client(), d.min_client());
        let (w, h) = d.client();
        let r = Rect::new(0, 0, w, h);
        let cs = ColorScheme::default();
        let p = Paint {
            text: u,
            ..classic_paint(&cs, 1)
        };
        let mut c = Canvas::new(w as u32, h as u32);
        d.draw(&mut c, &t, &p, r, 22);
        let lay = d.layout(&t, 22, r, &ControlMetrics::CLASSIC.grown(u));
        assert_eq!(
            lay.lookin.field,
            Rect::new(198, 16, lay.lookin.field.width, 44)
        );
        assert_eq!(lay.toolbar.height, toolbar::HEIGHT);
        assert_eq!(lay.toolbar.y, 16 + (44 - toolbar::HEIGHT) / 2);
        assert_eq!(
            lay.places,
            Rect::new(12, 74, 2 * placesbar::WIDTH, 2 * PLACES_H)
        );
        assert_eq!(lay.shell.frame.x, 2 * SHELL_X);
        assert_eq!(
            lay.filename,
            Rect::new(390, 788, w - 12 - 150 - 66 - 390, 44)
        );
        assert_eq!(lay.accept, Rect::new(w - 12 - 150, 788, 150, 46));
        assert_eq!(lay.cancel.y, 2 * FILETYPE_Y + CANCEL_DY);
        let (bx, by) = (2 * FIELD_X, 2 * EXTRA_Y);
        // The 13 px box centred down its 26 px row.
        assert_eq!(
            c.get(bx, by + 6),
            cs.button_shadow.argb(),
            "the extra check box's corner"
        );
        let hit = d.hit(&t, 22, r, bx + 5, by + 6 + 5);
        assert!(
            matches!(hit, Some(Hit::Extra(h)) if h.item == 0),
            "got {hit:?}"
        );
        let hit = d.hit(&t, 22, r, bx + 5, 2 * (EXTRA_Y + 20) + 5);
        assert!(
            matches!(hit, Some(Hit::Extra(h)) if h.item == 1),
            "got {hit:?}"
        );
    }

    #[test]
    fn a_grown_dialog_draws_its_corner_unchanged() {
        let (dw, dh) = (168, 55);
        let t = text();
        let cs = ColorScheme::default();
        let p = classic_paint(&cs, 1);
        let draw = |d: &FileDlg| {
            let (w, h) = d.client();
            let mut c = Canvas::new(w as u32, h as u32);
            d.draw(&mut c, &t, &p, Rect::new(0, 0, w, h), 11);
            c
        };
        let mut d = multi(DlgMode::Open, false);
        let small = draw(&d);
        d.set_size((CLIENT.0 + dw, CLIENT.1 + dh));
        let big = draw(&d);
        // From 20 px left of the accept button: past the fields' ends.
        let (x0, y0) = (CLIENT.0 - MARGIN - BUTTON_W - 20, FILENAME_Y - 4);
        for y in y0..CLIENT.1 {
            for x in x0..CLIENT.0 {
                assert_eq!(big.get(x + dw, y + dh), small.get(x, y), "at ({x}, {y})");
            }
        }
    }

    /// A press on the grip asks the host to resize; Cancel's right end,
    /// over the grip's square, stays Cancel's.
    #[test]
    fn the_size_grip_follows_the_corner_and_asks_for_a_resize() {
        let t = text();
        let mut d = sample();
        assert_eq!(
            d.hit(&t, 11, R, CLIENT.0 - 4, CLIENT.1 - 4),
            Some(Hit::Grip)
        );
        assert_eq!(
            d.hit(&t, 11, R, CLIENT.0 - 10, CLIENT.1 - 10),
            Some(Hit::Cancel)
        );
        d.set_size((CLIENT.0 + 100, CLIENT.1 + 100));
        let big = Rect::new(0, 0, CLIENT.0 + 100, CLIENT.1 + 100);
        let hit = d.hit(&t, 11, big, big.right() - 4, big.bottom() - 4);
        assert_eq!(hit, Some(Hit::Grip));
        assert!(matches!(d.click(big, hit, Mods::default()), Event::Resize));
    }

    fn sized(name: &str, size: u64) -> shellview::Entry {
        shellview::Entry { size, ..file(name) }
    }

    fn names(d: &FileDlg) -> Vec<&str> {
        d.entries().iter().map(|e| e.name.as_str()).collect()
    }

    /// A folder lists by Name, ascending, folders first. A click on a
    /// column title sorts by it, ascending; a second click on it reverses
    /// the whole order, files first. The selection stays on its entry.
    #[test]
    fn a_column_title_click_sorts_and_a_second_reverses() {
        let mut d = multi(DlgMode::Open, false);
        d.set_listing(folder(
            2,
            None,
            vec![
                sized("b.txt", 10),
                dir("Zeta"),
                sized("a.txt", 300),
                dir("alpha"),
                sized("C.txt", 20),
            ],
        ));
        assert_eq!(d.shell.sort, Some((0, true)));
        assert_eq!(names(&d), ["alpha", "Zeta", "a.txt", "b.txt", "C.txt"]);
        d.shell.selection.set_only(Some(3));
        let size = Some(Hit::Shell(shellview::Hit::Header(1)));
        d.click(R, size, Mods::default());
        d.release();
        assert_eq!(d.shell.sort, Some((1, true)));
        assert_eq!(names(&d), ["alpha", "Zeta", "b.txt", "C.txt", "a.txt"]);
        assert_eq!(d.shell.selection.focus, Some(2), "b.txt stays selected");
        d.click(R, size, Mods::default());
        d.release();
        assert_eq!(d.shell.sort, Some((1, false)));
        assert_eq!(names(&d), ["a.txt", "C.txt", "b.txt", "Zeta", "alpha"]);
        d.set_listing(folder(3, None, vec![dir("x"), sized("y", 1)]));
        assert_eq!(names(&d), ["y", "x"], "the next folder keeps the sort");
        assert_eq!(
            d.shell.selection.focus,
            Some(0),
            "and opens on its first entry"
        );
    }

    /// The extra rows are drawn as a shifted copy, yet their hit test
    /// still reads the look they were drawn in: after a Blue draw, the
    /// check box's label 16 px in hits it.
    #[test]
    fn a_blue_extra_check_box_is_hit_on_its_label_16_px_in() {
        let d = with_extra();
        let big = Rect::new(0, 0, CLIENT.0, CLIENT.1 + EXTRA_H);
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let mut c = Canvas::new(CLIENT.0 as u32, (CLIENT.1 + EXTRA_H) as u32);
        d.draw(&mut c, &text(), &theme.paint(), big, 11);
        let hit = d.hit(&text(), 11, big, FIELD_X + 16, EXTRA_Y + 5);
        assert!(
            matches!(hit, Some(Hit::Extra(h)) if h.item == 0),
            "the label 16 px in hits the check box, got {hit:?}"
        );
    }

    /// `hit` lays the dialog out at the sizes of its last draw: after a
    /// draw in a look with a 17 px scrollbar, the List view's bar starts
    /// 17 px up from the view's bottom, a row that is file list at 16.
    #[test]
    fn hit_follows_the_scrollbar_thickness_of_the_last_draw() {
        let mut d = multi(DlgMode::Open, false);
        let many = (0..60).map(|i| file(&format!("f{i}.txt"))).collect();
        d.set_listing(folder(1, None, many));
        let thick = ControlMetrics {
            scroll: 17,
            ..ControlMetrics::CLASSIC
        };
        let hb = d
            .shell_layout(R, &thick)
            .hscroll
            .expect("60 files overflow");
        assert_eq!(hb.height, 17);
        let (x, y) = (hb.x + 3, hb.y);
        let back = Some(Hit::Shell(shellview::Hit::HBar(BarHit::Back)));
        assert_eq!(
            d.hit(&text(), 11, R, x, y),
            Some(Hit::Shell(shellview::Hit::Background)),
            "Classic's 16 px bar starts a row lower"
        );
        use crate::look::styled::fixture::{one_part_style, solid_frames};
        use crate::style::{Layout as Stack, Part, PartImage, State};
        let caption = PartImage::new(
            solid_frames(&[crate::theme::Color::rgb(0, 0, 0)]),
            &[State::Normal],
            Stack::Vertical,
        );
        let style = one_part_style(Part::Caption, caption).with_controls(thick);
        let look = crate::look::Look::Styled(std::sync::Arc::new(style));
        let theme = Theme::default();
        let p = Paint {
            cs: &theme.colors,
            look: &look,
            s: 1,
            back: crate::look::Backdrop::Face,
            active: true,
            text: crate::theme::TextUnits::ONE,
        };
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        d.draw(&mut c, &text(), &p, R, 11);
        assert_eq!(d.hit(&text(), 11, R, x, y), back);
    }

    #[test]
    fn tab_walks_into_the_extra_rows_after_files_of_type() {
        let mut d = with_extra();
        d.focus = Focus::FileType;
        d.key(R, form::Key::Tab { back: false }, None, Mods::default());
        assert_eq!(d.focus, Focus::Extra);
        d.key(R, form::Key::Tab { back: false }, None, Mods::default());
        assert_eq!(d.focus, Focus::Extra, "second extra control");
        d.key(R, form::Key::Tab { back: false }, None, Mods::default());
        assert_eq!(d.focus, Focus::Accept);
    }

    #[test]
    fn extra_rows_pixels() {
        let d = with_extra();
        let (w, h) = d.client();
        let t = text();
        let theme = Theme::default();
        let cs = &theme.colors;
        let mut c = Canvas::new(w as u32, h as u32);
        d.draw(&mut c, &t, &classic_paint(cs, 1), Rect::new(0, 0, w, h), 11);
        let mut base = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        multi(DlgMode::Open, false).draw(&mut base, &t, &classic_paint(cs, 1), R, 11);
        for y in 0..CLIENT.1 {
            for x in 0..CLIENT.0 {
                let grip = x >= CLIENT.0 - 16 && y >= CLIENT.1 - 16;
                if (y >= EXTRA_Y - 1 && x >= FIELD_X - 1) || grip {
                    continue;
                }
                assert_eq!(
                    c.get(x, y),
                    base.get(x, y),
                    "pixel ({x}, {y}) outside the extra rows unchanged"
                );
            }
        }
        assert_eq!(
            c.get(FIELD_X, EXTRA_Y),
            cs.button_shadow.argb(),
            "check box edge at (FIELD_X, EXTRA_Y)"
        );
        assert_eq!(
            c.get(FIELD_X + 6, EXTRA_Y + 6),
            cs.window.argb(),
            "check box well"
        );
        assert_eq!(
            c.get(FIELD_X, EXTRA_Y + 20),
            cs.button_shadow.argb(),
            "combo edge 20 px lower"
        );
    }

    #[test]
    fn choice_combo_list_fits_the_grown_dialog() {
        let d = with_extra();
        let (_, h) = d.client();
        let it = d.extra().unwrap().item("extra-combo").unwrap();
        let form::Control::Combo(c) = &it.control else {
            panic!("combo")
        };
        assert!(
            combo::layout(c, it.rect, &ControlMetrics::CLASSIC)
                .list
                .bottom()
                <= h
        );
    }

    #[test]
    fn entering_a_folder_asks_for_a_listing() {
        let mut d = sample();
        d.set_listing(folder(1, None, vec![dir("Docs"), file("a.txt")]));
        let out = d.key(R, form::Key::Enter, None, Mods::default());
        assert!(
            matches!(out, Event::NeedListing(900)),
            "the entry's own id, not one this module minted: {out:?}"
        );
        assert_eq!(d.folder(), Some(1));
    }

    #[test]
    fn arrow_keys_step_the_listing_before_any_draw() {
        let mut d = sample();
        d.set_listing(folder(1, None, vec![dir("Docs"), file("a.txt")]));
        d.focus = Focus::Shell;
        d.shell.selection.set_only(Some(0));
        assert!(matches!(
            d.key(R, form::Key::Down, None, Mods::default()),
            Event::Redraw
        ));
        assert_eq!(d.shell.selection.single(), Some(1));
    }

    #[test]
    fn space_on_the_focused_accept_button_accepts_and_is_text_in_the_name_field() {
        let mut d = sample();
        d.set_listing(folder(1, None, vec![dir("Docs"), file("a.txt")]));
        d.set_file_name("a.txt");
        let space = || form::Key::Edit(EditKey::Insert(" ".into()));
        d.focus = Focus::FileName;
        assert!(matches!(
            d.key(R, space(), Some(' '), Mods::default()),
            Event::Redraw
        ));
        assert_eq!(
            d.file_name(),
            " ",
            "Space in the name field is text, typed over the selected name"
        );
        d.set_file_name("a.txt");
        d.focus = Focus::Accept;
        match d.key(R, space(), Some(' '), Mods::default()) {
            Event::Accept(names) => assert_eq!(names, vec!["a.txt"]),
            other => panic!("expected Accept([a.txt]), got {other:?}"),
        }
        match d.key(R, form::Key::Space, None, Mods::default()) {
            Event::Accept(names) => assert_eq!(names, vec!["a.txt"]),
            other => panic!("expected Accept([a.txt]) from a flat Space, got {other:?}"),
        }
    }

    #[test]
    fn enter_on_a_file_accepts_its_name() {
        let mut d = sample();
        d.set_listing(folder(1, None, vec![dir("Docs"), file("a.txt")]));
        d.set_file_name("a.txt");
        match d.key(R, form::Key::Enter, None, Mods::default()) {
            Event::Accept(names) => assert_eq!(names, vec!["a.txt"]),
            other => panic!("expected Accept, got {other:?}"),
        }
    }

    #[test]
    fn back_returns_to_the_previous_folder() {
        let mut d = sample();
        d.set_listing(folder(1, None, vec![dir("Docs")]));
        d.set_listing(folder(2, Some(1), vec![file("b.txt")]));
        assert_eq!(d.folder(), Some(2));
        match d.nav(Nav::Back) {
            Event::NeedListing(id) => assert_eq!(id, 1),
            other => panic!("expected NeedListing(1), got {other:?}"),
        }
    }

    #[test]
    fn relisting_the_shown_folder_adds_no_back_step() {
        let mut d = sample();
        d.set_listing(folder(1, None, vec![]));
        d.set_listing(folder(2, Some(1), vec![]));
        d.set_listing(folder(2, Some(1), vec![file("b.txt")]));
        assert!(
            matches!(d.nav(Nav::Back), Event::NeedListing(1)),
            "Back goes to folder 1"
        );
        assert!(
            matches!(d.nav(Nav::Back), Event::None),
            "one Back step only"
        );
    }

    #[test]
    fn forward_returns_to_the_folder_back_left_until_another_folder_is_entered() {
        let mut d = sample();
        d.set_listing(folder(1, None, vec![]));
        d.set_listing(folder(2, Some(1), vec![]));
        assert!(
            matches!(d.nav(Nav::Forward), Event::None),
            "nothing ahead yet"
        );
        assert!(matches!(d.nav(Nav::Back), Event::NeedListing(1)));
        d.set_listing(folder(1, None, vec![]));
        assert!(matches!(d.nav(Nav::Forward), Event::NeedListing(2)));
        d.set_listing(folder(2, Some(1), vec![]));
        assert!(
            matches!(d.nav(Nav::Back), Event::NeedListing(1)),
            "Forward keeps the Back step"
        );
        d.set_listing(folder(1, None, vec![]));
        d.set_listing(folder(3, Some(1), vec![]));
        assert!(
            matches!(d.nav(Nav::Forward), Event::None),
            "entering folder 3 drops folder 2 from Forward"
        );
    }

    #[test]
    fn backspace_goes_up_outside_the_file_name_field() {
        let mut d = sample();
        d.set_listing(folder(2, Some(1), vec![file("b.txt")]));
        let bs = || form::Key::Edit(EditKey::Backspace { word: false });
        d.focus = Focus::Shell;
        assert!(matches!(
            d.key(R, bs(), None, Mods::default()),
            Event::NeedListing(1)
        ));
        d.focus = Focus::FileName;
        d.set_file_name("b.txt");
        assert!(matches!(
            d.key(R, bs(), None, Mods::default()),
            Event::Redraw
        ));
        assert_eq!(d.file_name(), "", "the field deletes its selected name");
    }

    #[test]
    fn up_is_greyed_without_a_parent() {
        let mut d = sample();
        d.set_listing(folder(1, None, vec![]));
        assert!(!d.can_go_up());
        d.set_listing(folder(2, Some(1), vec![]));
        assert!(d.can_go_up());
    }

    #[test]
    fn escape_cancels() {
        let mut d = sample();
        assert!(matches!(
            d.key(R, form::Key::Escape, None, Mods::default()),
            Event::Cancel
        ));
    }

    #[test]
    fn set_toolbar_icons_fills_the_four_items_in_order() {
        let mut d = sample();
        let icon = || Some(Canvas::new(1, 1));
        d.set_toolbar_icons(icon(), None, icon(), None);
        let width_of = |item: &toolbar::Item| match item {
            toolbar::Item::Button { icon, .. } | toolbar::Item::DropButton { icon, .. } => {
                icon.as_ref().map(|c| c.width)
            }
            toolbar::Item::Separator => None,
        };
        let got: Vec<Option<u32>> = d.toolbar.items.iter().map(width_of).collect();
        assert_eq!(
            got,
            vec![Some(1), None, Some(1), Some(16)],
            "Back and New Folder the host's, Up none, Views the drawn 16 px"
        );
    }

    /// `combo_key` has no Escape arm, so Escape must not reach it.
    #[test]
    fn escape_closes_an_open_combo_instead_of_the_dialog() {
        let mut d = sample();
        d.lookin.open = Some(0);
        let ev = d.key(R, form::Key::Escape, None, Mods::default());
        assert!(matches!(ev, Event::Redraw), "got {ev:?}");
        assert!(d.lookin.open.is_none(), "the list closed");
    }

    /// `activate` would accept the File name field and end the dialog.
    #[test]
    fn enter_commits_an_open_combo_instead_of_accepting() {
        let mut d = sample();
        d.filename.set_selected("a.txt");
        d.lookin.open = Some(0);
        let ev = d.key(R, form::Key::Enter, None, Mods::default());
        assert!(
            !matches!(ev, Event::Accept(_) | Event::Cancel),
            "the dialog must not end while a list is dropped: {ev:?}"
        );
        assert!(d.lookin.open.is_none(), "the list closed");
    }

    /// A disabled tile is drawn (glyph kept, label greyed) but has nowhere to navigate.
    #[test]
    fn a_disabled_place_refuses_selection_and_activation() {
        let mut d = FileDlg::new(
            Fields {
                title: "Open".into(),
                accept_label: "Open".into(),
                mode: DlgMode::Open,
                multiple: false,
                filters: vec!["All Files (*.*)".into()],
            },
            vec![Place {
                label: "My Network Places".into(),
                icon: None,
                enabled: false,
            }],
        );
        assert!(matches!(
            d.click(R, Some(Hit::Places(0)), Mods::default()),
            Event::None
        ));
        assert!(
            d.places.selected.is_none(),
            "a greyed tile does not take the selection"
        );
        d.focus = Focus::Places;
        d.places.selected = Some(0);
        assert!(
            matches!(
                d.key(R, form::Key::Enter, None, Mods::default()),
                Event::None
            ),
            "and refuses activation even if something selected it"
        );
    }

    #[test]
    fn a_places_tile_asks_for_its_place() {
        let mut d = sample();
        let out = d.click(R, Some(Hit::Places(0)), Mods::default());
        assert!(matches!(out, Event::NeedPlace(0)), "got {out:?}");
        let out = d.key(R, form::Key::Enter, None, Mods::default());
        assert!(matches!(out, Event::NeedPlace(0)), "got {out:?}");
    }

    #[test]
    fn a_stale_entry_hit_is_ignored() {
        let mut d = sample();
        d.set_listing(folder(1, None, vec![file("a.txt"), file("b.txt")]));
        let hit = Some(Hit::Shell(shellview::Hit::Entry(1)));
        d.set_listing(folder(2, None, vec![]));
        d.click(R, hit, Mods::default());
        assert_eq!(d.shell.selection.single(), None);
        assert_eq!(d.file_name(), "");
        d.shell.selection.set_only(Some(1));
        assert!(matches!(
            d.key(R, form::Key::Enter, None, Mods::default()),
            Event::None
        ));
    }

    #[test]
    fn an_out_of_range_views_row_keeps_the_mode() {
        let mut d = sample();
        d.click(R, Some(Hit::ToolBar(TOOLBAR_VIEWS)), Mods::default());
        d.click(R, Some(Hit::ViewsItem(9)), Mods::default());
        assert_eq!(d.shell.mode, Mode::List);
        assert!(!d.views_open);
    }

    #[test]
    fn a_focused_glass_look_in_rings_right_of_its_icon() {
        use crate::interface::Interface;
        let text = text();
        let theme = crate::interfaces::theme_for(Interface::Glass);
        let mut d = sample();
        let mut f = folder(1, None, vec![]);
        let mut icon = Canvas::new(16, 16);
        icon.fill_rect(
            Rect::new(0, 0, 16, 16),
            crate::theme::Color::rgb(0x20, 0x80, 0x20),
        );
        f.crumbs[0].icon = Some(icon);
        d.set_listing(f);
        let draw = |d: &FileDlg| {
            let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
            d.draw(&mut c, &text, &theme.paint(), R, 11);
            c
        };
        let plain = draw(&d);
        d.key(R, form::Key::Tab { back: true }, None, Mods::default());
        assert_eq!(d.focus, Focus::LookIn);
        let focused = draw(&d);
        let field = d.layout(&text, 11, R, &theme.look.controls()).lookin.field;
        let changed: Vec<i32> = (field.y..field.bottom())
            .flat_map(|y| (field.x..field.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| plain.get(x, y) != focused.get(x, y))
            .map(|(x, _)| x)
            .collect();
        let left = changed.iter().copied().min().expect("a ring");
        let icon_right = field.x + LOOKIN_ICON_X + LOOKIN_ICON_PX;
        assert!(
            left > icon_right,
            "the ring starts at {left}, right of the icon"
        );
    }

    /// Under Blue every row sits a pixel higher than Classic's; the columns
    /// stay put and the places bar's tiles are 71 px.
    #[test]
    fn blues_rows_sit_a_pixel_higher() {
        let text = text();
        let mut d = multi(DlgMode::Open, false);
        let place = |l: &str| Place {
            label: l.into(),
            icon: None,
            enabled: true,
        };
        d.places.places = vec![place("Desktop"), place("My Computer")];
        let blue = crate::interfaces::blue_theme(crate::blue::Variant::Blue);
        let c = d.layout(&text, 11, R, &ControlMetrics::CLASSIC);
        let b = d.layout(&text, 11, R, &blue.look.controls());
        let up = |r: Rect| Rect::new(r.x, r.y - 1, r.width, r.height);
        assert_eq!(
            b.lookin.field,
            Rect::new(R.x + LOOKIN_X, 7, c.lookin.field.width, 22)
        );
        assert_eq!(b.lookin_label, up(c.lookin_label));
        assert_eq!(b.toolbar, up(c.toolbar));
        assert_eq!(b.places, Rect::new(MARGIN, 36, placesbar::WIDTH, PLACES_H));
        assert_eq!(
            b.places_rects[1],
            Rect::new(MARGIN, 36 + 71, placesbar::WIDTH, 71)
        );
        assert_eq!(b.shell.frame, up(c.shell.frame));
        assert_eq!(b.filename, up(c.filename));
        assert_eq!(b.filename_label, up(c.filename_label));
        assert_eq!(b.filetype.field, up(c.filetype.field));
        assert_eq!(b.filetype_label, up(c.filetype_label));
        assert_eq!(b.accept, up(c.accept));
        assert_eq!(b.cancel, up(c.cancel));
    }

    #[test]
    fn tab_wraps_the_focus_order() {
        let mut d = sample();
        assert_eq!(d.focus, Focus::FileName);
        d.key(R, form::Key::Tab { back: true }, None, Mods::default());
        assert_eq!(d.focus, Focus::LookIn);
        d.key(R, form::Key::Tab { back: false }, None, Mods::default());
        assert_eq!(d.focus, Focus::FileName);
        d.key(R, form::Key::Tab { back: false }, None, Mods::default());
        assert_eq!(d.focus, Focus::FileType);
    }

    #[test]
    fn arrow_keys_skip_a_disabled_place() {
        let place = |enabled| Place {
            label: "Place".into(),
            icon: None,
            enabled,
        };
        let mut d = FileDlg::new(
            Fields {
                title: "Open".into(),
                accept_label: "Open".into(),
                mode: DlgMode::Open,
                multiple: false,
                filters: vec!["All Files (*.*)".into()],
            },
            vec![place(false), place(true), place(false), place(true)],
        );
        d.focus = Focus::Places;
        let key = |d: &mut FileDlg, k| {
            d.key(R, k, None, Mods::default());
            d.places.selected
        };
        assert_eq!(key(&mut d, form::Key::Down), Some(1));
        assert_eq!(key(&mut d, form::Key::Down), Some(3));
        assert_eq!(key(&mut d, form::Key::Down), Some(3));
        assert_eq!(key(&mut d, form::Key::Up), Some(1));
        assert_eq!(key(&mut d, form::Key::Up), Some(1));
    }

    /// Left, Right, Home and End arrive as edit keys and walk the view's cells as the arrows do.
    #[test]
    fn edit_arrow_keys_walk_the_icon_view() {
        let mut d = multi(DlgMode::Open, false);
        let names: Vec<String> = (0..12).map(|i| format!("f{i:02}.txt")).collect();
        d.set_listing(folder(1, None, names.iter().map(|n| file(n)).collect()));
        d.shell.set_mode(Mode::Icons);
        d.focus = Focus::Shell;
        d.shell.selection.set_only(Some(0));
        let mut key = |k: EditKey| {
            d.key(R, form::Key::Edit(k), None, Mods::default());
            d.shell.selection.focus
        };
        assert_eq!(
            key(EditKey::Right {
                select: false,
                word: false
            }),
            Some(1)
        );
        assert_eq!(
            key(EditKey::Left {
                select: false,
                word: false
            }),
            Some(0)
        );
        assert_eq!(key(EditKey::End { select: false }), Some(11));
        assert_eq!(key(EditKey::Home { select: false }), Some(0));
    }

    #[test]
    fn down_on_an_empty_places_bar_does_not_panic() {
        let mut d = FileDlg::new(
            Fields {
                title: "Open".into(),
                accept_label: "Open".into(),
                mode: DlgMode::Open,
                multiple: false,
                filters: vec!["All Files (*.*)".into()],
            },
            Vec::new(),
        );
        d.focus = Focus::Places;
        d.places.selected = Some(0);
        let _ = d.key(R, form::Key::Down, None, Mods::default());
    }

    #[test]
    fn the_wheel_scrolls_the_view_and_an_open_list() {
        let filters: Vec<String> = (0..40).map(|i| format!("Type {i}")).collect();
        let mut d = FileDlg::new(
            Fields {
                title: "Open".into(),
                accept_label: "Open".into(),
                filters,
                mode: DlgMode::Open,
                multiple: false,
            },
            vec![],
        );
        let names: Vec<String> = (0..60).map(|i| format!("f{i:02}.txt")).collect();
        d.set_listing(folder(1, None, names.iter().map(|n| file(n)).collect()));
        d.shell.set_mode(Mode::Icons);
        let view = Some(Hit::Shell(shellview::Hit::Background));
        assert!(matches!(d.wheel(R, view, WHEEL_ROWS), Event::Redraw));
        assert!(d.shell.top > 0, "the view scrolled down");
        assert!(matches!(d.wheel(R, view, -WHEEL_ROWS), Event::Redraw));
        assert_eq!(d.shell.top, 0, "and back up");
        assert!(matches!(d.wheel(R, view, -WHEEL_ROWS), Event::None));
        assert!(matches!(
            d.wheel(R, Some(Hit::Accept), WHEEL_ROWS),
            Event::None
        ));
        assert!(matches!(d.wheel(R, None, WHEEL_ROWS), Event::None));

        let on_type = Some(Hit::FileType(combo::Hit::Field));
        assert!(
            matches!(d.wheel(R, on_type, WHEEL_ROWS), Event::None),
            "an unfocused Files of type ignores the wheel"
        );
        d.click(R, Some(Hit::FileType(combo::Hit::Drop)), Mods::default());
        assert!(matches!(d.wheel(R, view, WHEEL_ROWS), Event::Redraw));
        assert_eq!(
            d.filetype.top, WHEEL_ROWS as usize,
            "the open list scrolled"
        );
        assert_eq!(d.shell.top, 0, "the view under the pointer stayed");
        assert_eq!(d.filetype.selected, 0, "scrolling picks nothing");

        d.click(R, Some(Hit::FileType(combo::Hit::Drop)), Mods::default());
        assert!(d.filetype.open.is_none());
        assert!(matches!(
            d.wheel(R, on_type, WHEEL_ROWS),
            Event::FilterChanged(1)
        ));
        assert!(matches!(
            d.wheel(R, on_type, -2 * WHEEL_ROWS),
            Event::FilterChanged(0)
        ));
    }

    fn name_field(d: &FileDlg, t: &TextRenderer, px: u32) -> Rect {
        d.layout(t, px, R, &d.drawn.0.get()).filename
    }

    #[test]
    fn text_dropped_on_file_name_lands_at_the_drop_caret_and_takes_the_focus() {
        let (t, px) = (text(), Theme::default().fonts.size_px);
        let mut d = sample();
        d.set_file_name("ab.txt");
        d.focus = Focus::Shell;
        let f = name_field(&d, &t, px);
        let x = f.x + widgets::FIELD_PAD + t.measure("ab", false, px);
        assert!(
            !d.drag_over(&t, px, R, f.x, f.y - 20),
            "the view takes no text"
        );
        assert!(d.drag_over(&t, px, R, x, f.y + 5));
        assert!(d.drop_text(&t, px, R, &crate::textedit::DropLine::Text("c".into())));
        assert_eq!(d.file_name(), "abc.txt");
        assert_eq!(d.filename.selected_text(), "c");
        assert_eq!(d.focus, Focus::FileName);
        assert!(
            !d.drop_text(&t, px, R, &crate::textedit::DropLine::Text("x".into())),
            "the drop ended the drag"
        );
        assert_eq!(d.typed, "abc.txt", "the dropped text counts as typed");
    }

    #[test]
    fn dropped_files_replace_file_name_quoted_only_where_several_are_picked() {
        let (t, px) = (text(), Theme::default().fonts.size_px);
        let names = ["a.txt".to_string(), "/tmp/my b.txt".to_string()];
        let mut one = sample();
        one.set_file_name("old");
        let f = name_field(&one, &t, px);
        assert!(!one.drop_names(&names), "no drag rests on the field");
        assert!(one.drag_over(&t, px, R, f.x + 10, f.y + 5));
        assert!(one.drop_names(&names));
        assert_eq!(one.file_name(), "a.txt");
        assert_eq!(one.filename.selected_text(), "a.txt");
        let mut many = multi(DlgMode::Open, true);
        assert!(many.drag_over(&t, px, R, f.x + 10, f.y + 5));
        assert!(many.drop_names(&names));
        assert_eq!(many.file_name(), r#""a.txt" "/tmp/my b.txt""#);
        assert_eq!(
            split_names(many.file_name()),
            names,
            "Accept takes them back apart"
        );
    }

    #[test]
    fn a_drag_resting_at_file_names_edge_scrolls_it_and_draws_a_drop_caret() {
        let (t, theme) = (text(), Theme::default());
        let px = theme.fonts.size_px;
        let mut d = sample();
        d.set_file_name(&"long name ".repeat(12));
        let f = name_field(&d, &t, px);
        let (y, left) = (f.y + 5, f.x + 2);
        assert!(d.drag_over(&t, px, R, f.x + 40, y));
        assert_eq!(d.drag_scroll(&t, px, R), None, "the middle rests still");
        let end = d.name_drop.unwrap().1;
        assert!(end > 0, "the caret at the end scrolled the name");
        assert!(d.drag_over(&t, px, R, left, y));
        assert!(d.drag_tick(&t, px, R));
        assert_eq!(d.name_drop, Some((left, end - 1)));
        let shot = |d: &FileDlg| {
            let mut c = Canvas::new(R.width as u32, R.height as u32);
            d.draw(&mut c, &t, &classic_paint(&theme.colors, 1), R, px);
            c
        };
        let dropping = shot(&d);
        assert!(d.drag_leave());
        assert!(!d.drag_tick(&t, px, R), "no drag, no step");
        assert_ne!(dropping.pixels, shot(&d).pixels);
    }
}
