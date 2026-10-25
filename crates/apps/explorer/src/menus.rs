//! The menus: the six on the menu bar, the item, background and tree context
//! menus, their submenus and the Views button's drop. Rows with no Linux
//! counterpart are drawn greyed.
//!
//! Every menu is a [`Menu`]: the rows and each row's [`Command`], built together
//! so they cannot drift. A row's address is an index path: `[9]` for a row of
//! the menu, `[9, 2]` for the third row of its submenu.

use crate::commands::{Command, Menu};
use crate::place::Place;
use reprisefs::ops::DropOp;
use reprisefs::program::{Program, ScriptOpen};
use reprisekit::controls::menu::MenuItem;
use reprisekit::controls::menubar::Bar;
use reprisekit::controls::shellview::Mode;
use reprisekit::raster::Canvas;

#[derive(Clone, Debug, PartialEq)]
pub struct MenuState {
    pub mode: Mode,
    /// The Details sort column and direction; Arrange Icons by bullets the column.
    pub sort: (usize, bool),
    pub selection: usize,
    pub can_paste: bool,
    /// The verb of the undoable action ("Delete", "Move"), for Edit's first row;
    /// `None` greys it.
    pub can_undo: Option<&'static str>,
    pub folders: bool,
    pub status_bar: bool,
    pub toolbars: [bool; 3],
    pub read_only: bool,
    /// The Recycle Bin is up, holding this many items: File and the item menu
    /// trade their file rows for its own.
    pub bin: Option<usize>,
    pub can_back: bool,
    pub can_forward: bool,
}

impl Default for MenuState {
    fn default() -> MenuState {
        MenuState {
            mode: Mode::Tiles,
            sort: (0, true),
            selection: 0,
            can_paste: false,
            can_undo: None,
            folders: true,
            status_bar: true,
            toolbars: [true, true, false],
            read_only: false,
            bin: None,
            can_back: false,
            can_forward: false,
        }
    }
}

fn row(label: &str, enabled: bool) -> MenuItem {
    MenuItem::item(label).enabled(enabled)
}

fn keyed(label: &str, enabled: bool, shortcut: &str) -> MenuItem {
    with(row(label, enabled), |e| {
        if let MenuItem::Entry { shortcut: s, .. } = e {
            *s = Some(shortcut.to_string());
        }
    })
}

fn check(label: &str, enabled: bool, on: bool) -> MenuItem {
    with(row(label, enabled), |e| {
        if let MenuItem::Entry { checked, .. } = e {
            *checked = on;
        }
    })
}

fn bullet(label: &str, on: bool) -> MenuItem {
    with(row(label, true), |e| {
        if let MenuItem::Entry { checked, radio, .. } = e {
            *checked = on;
            *radio = true;
        }
    })
}

fn bold(label: &str, enabled: bool) -> MenuItem {
    with(row(label, enabled), |e| {
        if let MenuItem::Entry { bold, .. } = e {
            *bold = true;
        }
    })
}

fn with_icon(item: MenuItem, icon: Option<Canvas>) -> MenuItem {
    with(item, |e| {
        if let MenuItem::Entry { icon: i, .. } = e {
            *i = icon;
        }
    })
}

fn with(mut item: MenuItem, f: impl FnOnce(&mut MenuItem)) -> MenuItem {
    f(&mut item);
    item
}

/// Edit's and the background menu's first row: "Undo Delete" when there
/// is something to undo, a greyed "Undo" when there is not.
fn undo_row(state: &MenuState) -> MenuItem {
    match state.can_undo {
        Some(verb) => keyed(&format!("&Undo {verb}"), true, "Ctrl+Z"),
        None => keyed("&Undo", false, "Ctrl+Z"),
    }
}

/// The five view modes in order, with their access keys.
const MODES: [(&str, Mode); 5] = [
    ("T&humbnails", Mode::Thumbnails),
    ("Tile&s", Mode::Tiles),
    ("Ico&ns", Mode::Icons),
    ("&List", Mode::List),
    ("&Details", Mode::Details),
];

/// The Details columns the Arrange Icons by submenu sorts on, and the
/// Recycle Bin's.
const SORT_COLUMNS: [&str; 4] = ["&Name", "Si&ze", "T&ype", "Modifie&d"];
const BIN_SORT_COLUMNS: [&str; 6] = [
    "&Name",
    "&Original Location",
    "&Date Deleted",
    "&Size",
    "&Type",
    "&Modified",
];

fn new_menu(writable: bool) -> Menu {
    let mut m = Menu::default();
    m.row(row("&Folder", writable), Command::NewFolder);
    m.row(row("&Shortcut", false), Command::Nothing);
    m.sep();
    m.row(row("&Text Document", writable), Command::NewTextDocument);
    m
}

/// View > Toolbars. Links has nothing behind it and stays greyed, as do
/// the two rows below the separator.
fn toolbars_menu(state: &MenuState) -> Menu {
    let mut m = Menu::default();
    m.row(
        check("&Standard Buttons", true, state.toolbars[0]),
        Command::ToggleToolbar(0),
    );
    m.row(
        check("&Address Bar", true, state.toolbars[1]),
        Command::ToggleToolbar(1),
    );
    m.row(check("&Links", false, state.toolbars[2]), Command::Nothing);
    m.sep();
    m.row(check("Loc&k the Toolbars", false, false), Command::Nothing);
    m.row(row("&Customize...", false), Command::Nothing);
    m
}

/// View > Explorer Bar: Folders. Favorites, History and the Tip of the
/// Day have no counterpart.
fn explorer_bar_menu(state: &MenuState) -> Menu {
    let mut m = Menu::default();
    m.row(keyed("&Favorites", false, "Ctrl+I"), Command::Nothing);
    m.row(keyed("&History", false, "Ctrl+H"), Command::Nothing);
    m.row(
        check("F&olders", true, state.folders),
        Command::ToggleFolders,
    );
    m.sep();
    m.row(row("Tip of the &Day", false), Command::Nothing);
    m
}

/// View's and the background menu's Arrange Icons by: the sort columns
/// as one radio group, then the three grid rows, drawn greyed.
fn arrange_menu(state: &MenuState) -> Menu {
    let mut m = Menu::default();
    let labels: &[&str] = if state.bin.is_some() {
        &BIN_SORT_COLUMNS
    } else {
        &SORT_COLUMNS
    };
    for (i, label) in labels.iter().enumerate() {
        m.row(bullet(label, state.sort.0 == i), Command::Sort(i));
    }
    m.sep();
    m.row(row("Show in &Groups", false), Command::Nothing);
    m.row(check("Auto &Arrange", false, false), Command::Nothing);
    m.row(row("Align to Gri&d", false), Command::Nothing);
    m
}

fn go_to_menu(state: &MenuState) -> Menu {
    let mut m = Menu::default();
    m.row(keyed("&Back", state.can_back, "Alt+Left"), Command::Back);
    m.row(
        keyed("&Forward", state.can_forward, "Alt+Right"),
        Command::Forward,
    );
    m.row(row("&Up One Level", true), Command::Up);
    m.sep();
    m.row(row("&Home Page", false), Command::Nothing);
    m.sep();
    m.row(row("&Documents", true), Command::GoTo(Place::Documents));
    m.row(row("Ho&me", true), Command::GoTo(Place::Home));
    m.row(row("&Computer", true), Command::GoTo(Place::Computer));
    m
}

/// The five modes as one radio group: the View menu's own rows are flat,
/// the background menu's View submenu and the Views drop are this.
fn modes_menu(mode: Mode) -> Menu {
    let mut m = Menu::default();
    for (label, want) in MODES {
        m.row(bullet(label, mode == want), Command::Mode(want));
    }
    m
}

/// The item menu's Open With: the programs registered for the file, then
/// the chooser.
fn open_with_menu(open_with: &[(String, Option<Canvas>)]) -> Menu {
    let mut m = Menu::default();
    for (i, (name, icon)) in open_with.iter().enumerate() {
        m.row(
            with_icon(MenuItem::entry(name.as_str()), icon.clone()),
            Command::OpenWith(i),
        );
    }
    if !open_with.is_empty() {
        m.sep();
    }
    m.row(row("C&hoose Program...", true), Command::ChooseProgram);
    m
}

/// Send To: one row per target with its icon, greyed where nothing is
/// behind it (Desktop without a desktop folder).
fn send_to_menu(send_to: &[(String, bool, Option<Canvas>)]) -> Menu {
    let mut m = Menu::default();
    for (i, (name, enabled, icon)) in send_to.iter().enumerate() {
        m.row(
            with_icon(
                MenuItem::entry(name.as_str()).enabled(*enabled),
                icon.clone(),
            ),
            Command::SendTo(i),
        );
    }
    m
}

/// The six menus of the bar with their commands, greyed and checked for
/// `state`. [`menu_bar`] draws them and [`command_for`] resolves a row.
fn bar_menus(state: &MenuState) -> Vec<(&'static str, Menu)> {
    let sel = state.selection > 0;
    let writable = sel && !state.read_only;

    let mut file = Menu::default();
    match state.bin {
        Some(_) if sel => file.row(row("R&estore", true), Command::Restore),
        Some(n) => file.row(row("&Empty Recycle Bin", n > 0), Command::EmptyRecycleBin),
        None => file.sub(row("Ne&w", !state.read_only), new_menu(!state.read_only)),
    }
    file.sep();
    file.row(row("Create &Shortcut", writable), Command::CreateShortcut);
    // An item in the Recycle Bin can only be deleted for good.
    let (delete, erase) = match state.bin {
        Some(_) => (sel, true),
        None => (writable, false),
    };
    file.row(row("&Delete", delete), Command::Delete { erase });
    file.row(row("Rena&me", writable), Command::Rename);
    // With nothing selected, Properties is the place's own sheet, which
    // Computer and the Recycle Bin do not have
    // (`props_host::place_sheet`).
    file.row(
        row("P&roperties", sel || !state.read_only),
        Command::Properties,
    );
    file.sep();
    file.row(row("&Close", true), Command::Close);

    let mut edit = Menu::default();
    edit.row(undo_row(state), Command::Undo);
    edit.sep();
    edit.row(keyed("Cu&t", writable, "Ctrl+X"), Command::Cut);
    // An item in the Recycle Bin is not copied out; Restore brings it
    // back.
    edit.row(
        keyed("&Copy", sel && state.bin.is_none(), "Ctrl+C"),
        Command::Copy,
    );
    let pasteable = state.can_paste && !state.read_only;
    edit.row(keyed("&Paste", pasteable, "Ctrl+V"), Command::Paste);
    edit.row(row("Paste &Shortcut", pasteable), Command::PasteShortcut);
    edit.sep();
    edit.row(row("Copy To Folder...", false), Command::Nothing);
    edit.row(row("Move To Folder...", false), Command::Nothing);
    edit.sep();
    edit.row(keyed("Select &All", true, "Ctrl+A"), Command::SelectAll);
    edit.row(row("&Invert Selection", true), Command::InvertSelection);

    let mut view = Menu::default();
    view.sub(row("&Toolbars", true), toolbars_menu(state));
    view.row(
        check("Status &Bar", true, state.status_bar),
        Command::ToggleStatusBar,
    );
    view.sub(row("&Explorer Bar", true), explorer_bar_menu(state));
    view.sep();
    for (label, mode) in MODES {
        view.row(bullet(label, state.mode == mode), Command::Mode(mode));
    }
    view.sep();
    view.sub(row("Arrange &Icons by", true), arrange_menu(state));
    view.sep();
    view.row(row("C&hoose Details...", false), Command::Nothing);
    view.sub(row("&Go To", true), go_to_menu(state));
    view.row(row("R&efresh", true), Command::Refresh);

    let mut favorites = Menu::default();
    favorites.row(row("&Add to Favorites...", false), Command::Nothing);
    favorites.row(row("&Organize Favorites...", false), Command::Nothing);
    favorites.sep();
    favorites.sub(row("Favorites &Bar", false), Menu::default());
    favorites.sub(row("&Web Sites", false), Menu::default());

    let mut tools = Menu::default();
    tools.row(row("&Synchronize...", false), Command::Nothing);
    tools.sep();
    tools.row(row("&Folder Options...", false), Command::Nothing);

    let mut help = Menu::default();
    // There is no help viewer: the row is drawn and does nothing.
    help.row(row("&Help and Support", true), Command::Nothing);
    help.sep();
    help.row(row("&About Explorer", true), Command::About);

    vec![
        ("&File", file),
        ("&Edit", edit),
        ("&View", view),
        ("F&avorites", favorites),
        ("&Tools", tools),
        ("&Help", help),
    ]
}

pub fn menu_bar(state: &MenuState) -> Bar {
    let mut bar = Bar::with_trees(
        bar_menus(state)
            .iter()
            .map(|(title, menu)| (*title, menu.tree()))
            .collect(),
    );
    bar.banded = true;
    bar
}

pub fn command_for(title: usize, path: &[usize], state: &MenuState) -> Command {
    match bar_menus(state).get(title) {
        Some((_, menu)) => menu.command(path),
        None => Command::Nothing,
    }
}

pub fn views_menu(mode: Mode) -> Menu {
    modes_menu(mode)
}

/// What an entry in the view is, for its right-click menu.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ItemKind {
    Folder,
    File,
    /// An executable script, whose menu runs it as its default ([`ScriptOpen`]) says.
    Script(ScriptOpen),
    /// A trusted program launcher (`reprisefs::apps::Shortcut::App`).
    Launcher,
}

impl ItemKind {
    pub(crate) fn is_file(self) -> bool {
        matches!(self, ItemKind::File | ItemKind::Script(_))
    }
}

/// The right-click menu of an entry in the view. A script has Run and Run in terminal in place
/// of Open. "Run as..." sits greyed under them for every file. A folder has Paste under Copy, into it. A launcher has
/// no Open With and has Paste under Copy, opening the clipboard's files in its
/// program. `eject` adds Eject, in a group above Cut and Copy, for a removable drive.
pub fn item_menu(
    state: &MenuState,
    kind: ItemKind,
    eject: bool,
    open_with: &[(String, Option<Canvas>)],
    send_to: &[(String, bool, Option<Canvas>)],
) -> Menu {
    if state.bin.is_some() {
        return bin_item_menu();
    }
    let writable = !state.read_only;
    let mut m = Menu::default();
    match kind {
        ItemKind::Script(open) => {
            for (o, default) in reprisefs::program::run_rows(Program::Script, open).unwrap() {
                let label = o.label();
                m.row(
                    if default {
                        bold(label, true)
                    } else {
                        row(label, true)
                    },
                    Command::Run(o),
                );
            }
        }
        _ => m.row(bold("&Open", true), Command::Open),
    }
    if kind == ItemKind::Folder {
        m.row(row("E&xplore", true), Command::Explore);
        m.row(row("Sharing and Security...", false), Command::Nothing);
    } else {
        m.row(row("Run &as...", false), Command::Nothing);
        m.sep();
        if kind.is_file() {
            m.sub(row("Open Wit&h", true), open_with_menu(open_with));
        }
    }
    m.sub(
        row("Se&nd To", !send_to.is_empty() && writable),
        send_to_menu(send_to),
    );
    if eject {
        m.sep();
        m.row(row("E&ject", true), Command::Eject);
    }
    m.sep();
    m.row(row("Cu&t", writable), Command::Cut);
    m.row(row("&Copy", true), Command::Copy);
    match kind {
        ItemKind::Folder => m.row(
            row("&Paste", state.can_paste && writable),
            Command::PasteOnItem,
        ),
        ItemKind::Launcher => m.row(row("&Paste", state.can_paste), Command::PasteOnItem),
        ItemKind::File | ItemKind::Script(_) => {}
    }
    m.sep();
    m.row(row("Create &Shortcut", writable), Command::CreateShortcut);
    m.row(row("&Delete", writable), Command::Delete { erase: false });
    m.row(row("Rena&me", writable), Command::Rename);
    m.sep();
    m.row(row("P&roperties", true), Command::Properties);
    m
}

/// The right-click menu of an item in the Recycle Bin: Restore, Cut (greyed),
/// Delete for good, then Properties as the default row.
fn bin_item_menu() -> Menu {
    let mut m = Menu::default();
    m.row(row("R&estore", true), Command::Restore);
    m.sep();
    m.row(row("Cu&t", false), Command::Nothing);
    m.sep();
    m.row(row("&Delete", true), Command::Delete { erase: true });
    m.sep();
    m.row(bold("P&roperties", true), Command::Properties);
    m
}

pub fn background_menu(state: &MenuState) -> Menu {
    let writable = !state.read_only;
    let pasteable = state.can_paste && writable;
    let mut m = Menu::default();
    m.sub(row("&View", true), modes_menu(state.mode));
    m.sep();
    m.sub(row("Arrange &Icons By", true), arrange_menu(state));
    m.row(row("R&efresh", true), Command::Refresh);
    m.sep();
    m.row(row("C&ustomize This Folder...", false), Command::Nothing);
    m.sep();
    m.row(row("&Paste", pasteable), Command::Paste);
    m.row(row("Paste &Shortcut", pasteable), Command::PasteShortcut);
    m.row(undo_row(state), Command::Undo);
    m.sep();
    m.sub(row("Ne&w", writable), new_menu(writable));
    m.sep();
    // The background's Properties is the place's own sheet; the three
    // places with no folder behind them have none.
    m.row(row("P&roperties", writable), Command::Properties);
    m
}

/// The right-click menu of a row in the Folders tree. Its first row is the
/// node's default action: Collapse, Expand, or greyed when nothing is to show.
pub fn tree_menu(expanded: bool, has_children: bool, folder: bool) -> Menu {
    let mut m = Menu::default();
    let label = if expanded { "&Collapse" } else { "&Expand" };
    m.row(bold(label, has_children), Command::ToggleNode);
    m.row(row("E&xplore", true), Command::OpenNode);
    m.row(row("&Open", true), Command::OpenNode);
    m.sep();
    m.row(row("Sharing and Security...", false), Command::Nothing);
    m.sub(row("Se&nd To", false), send_to_menu(&[]));
    m.sep();
    // The file rows act on the node itself, so they are live only for a
    // plain folder; a shell place or a drive is not a file to cut.
    m.row(row("Cu&t", folder), Command::NodeCut);
    m.row(row("&Copy", folder), Command::NodeCopy);
    m.sep();
    m.row(row("&Delete", folder), Command::NodeDelete);
    // Rename needs the tree's own label editor, which nothing draws yet.
    m.row(row("Rena&me", false), Command::Nothing);
    m.sep();
    m.row(row("P&roperties", folder), Command::NodeProperties);
    m
}

/// The menu a drop that asks opens (the right button, or a shortcut drag) over
/// the folder it landed on: Copy Here, Move Here, Create Shortcuts Here, a
/// separator and Cancel. `default` is drawn bold; a row the dragging program's
/// `source` actions cannot settle is greyed.
pub fn drop_menu(default: DropOp, source: repriseapp::Actions) -> Menu {
    let item = |label: &str, op: DropOp, enabled: bool| {
        if op == default {
            bold(label, enabled)
        } else {
            row(label, enabled)
        }
    };
    let mut m = Menu::default();
    m.row(
        item("&Copy Here", DropOp::Copy, source.copy),
        Command::Drop(DropOp::Copy),
    );
    m.row(
        item("&Move Here", DropOp::Move, source.move_),
        Command::Drop(DropOp::Move),
    );
    m.row(
        item(
            "Create &Shortcuts Here",
            DropOp::Shortcut,
            source.copy || source.move_,
        ),
        Command::Drop(DropOp::Shortcut),
    );
    m.sep();
    m.row(row("Cancel", true), Command::DropCancel);
    m
}

#[cfg(test)]
#[path = "menus_tests.rs"]
mod tests;
