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
use reprisekit::text::escape_mnemonic;
use reprisekit::tr;

#[derive(Clone, Debug, PartialEq)]
pub struct MenuState {
    pub mode: Mode,
    /// The Details sort column and direction; Arrange Icons by bullets the column.
    pub sort: (usize, bool),
    pub selection: usize,
    /// How many of the selected are the Desktop's rows for the shell places, which the file rows
    /// leave out.
    pub shell_rows: usize,
    /// The selection is only Desktop rows and Properties has no sheet for it.
    pub sheetless: bool,
    /// The clipboard holds files.
    pub can_paste: bool,
    /// It holds a picture alone, which Paste saves as a file and Paste Shortcut cannot link.
    pub can_paste_image: bool,
    /// The verb of the undoable action ("Delete", "Move"), for Edit's first row;
    /// `None` greys it.
    pub can_undo: Option<&'static str>,
    pub folders: bool,
    pub status_bar: bool,
    /// View > Hidden Items: dot names and those a folder's `.hidden` lists are shown.
    pub hidden: bool,
    pub toolbars: [bool; 3],
    pub read_only: bool,
    /// The Recycle Bin is up, holding this many items: File and the item menu
    /// trade their file rows for its own.
    pub bin: Option<usize>,
    pub can_back: bool,
    pub can_forward: bool,
    /// The selection is archives: the item menu's Extract to row names this folder.
    pub extract: Option<String>,
    /// Every selected item has a local path, for Copy as Path.
    pub local_paths: bool,
    /// The zip the item menu's Compress to row makes; `None` hides the row.
    pub compress: Option<String>,
    /// The selection is one disk image: the item menu's default row mounts it.
    pub disk_image: bool,
    /// The selection is one drive whose label Rename sets.
    pub relabel: bool,
    /// A lone folder under the item menu: whether it is pinned (Unpin from Places) or not (Pin
    /// to Places).
    pub pinned: Option<bool>,
    /// The selection is one removable drive, disk image, phone or network share: File and the
    /// item menu show Eject.
    pub eject: bool,
    /// The selection is a network share: its Eject row reads Disconnect.
    pub share: bool,
    /// gio is there to connect with: File's Connect to Server... is live.
    pub can_connect: bool,
    /// The Templates folder's files, New's rows under Text Document.
    pub templates: Vec<reprisefs::templates::Template>,
    /// The place shown is a folder: Add to Favorites... is live.
    pub can_add_favorite: bool,
    /// The Favorites menu's rows under its separator.
    pub favorites: Vec<crate::favorites::MenuRow>,
}

impl Default for MenuState {
    fn default() -> MenuState {
        MenuState {
            mode: Mode::Tiles,
            sort: (0, true),
            selection: 0,
            shell_rows: 0,
            sheetless: false,
            can_paste: false,
            can_paste_image: false,
            can_undo: None,
            folders: true,
            status_bar: true,
            hidden: false,
            toolbars: [true, true, false],
            read_only: false,
            bin: None,
            can_back: false,
            can_forward: false,
            extract: None,
            local_paths: false,
            compress: None,
            disk_image: false,
            relabel: false,
            pinned: None,
            eject: false,
            share: false,
            can_connect: true,
            templates: Vec::new(),
            can_add_favorite: false,
            favorites: Vec::new(),
        }
    }
}

/// Shown on a folder and on the background of a place with a folder on disk, which Computer and
/// the Recycle Bin are not.
fn terminal() -> String {
    tr!("menu-open-in-terminal")
}

fn row(label: impl AsRef<str>, enabled: bool) -> MenuItem {
    MenuItem::item(label.as_ref()).enabled(enabled)
}

fn keyed(label: impl AsRef<str>, enabled: bool, shortcut: &str) -> MenuItem {
    with(row(label, enabled), |e| {
        if let MenuItem::Entry { shortcut: s, .. } = e {
            *s = Some(shortcut.to_string());
        }
    })
}

fn check(label: impl AsRef<str>, enabled: bool, on: bool) -> MenuItem {
    checked(row(label, enabled), on)
}

fn checked(item: MenuItem, on: bool) -> MenuItem {
    with(item, |e| {
        if let MenuItem::Entry { checked, .. } = e {
            *checked = on;
        }
    })
}

fn bullet(label: impl AsRef<str>, on: bool) -> MenuItem {
    with(row(label, true), |e| {
        if let MenuItem::Entry { checked, radio, .. } = e {
            *checked = on;
            *radio = true;
        }
    })
}

fn bold(label: impl AsRef<str>, enabled: bool) -> MenuItem {
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
        Some(verb) => keyed(tr!("menu-undo-row-undo-verb", verb = verb), true, "Ctrl+Z"),
        None => keyed(tr!("menu-undo-row-undo"), false, "Ctrl+Z"),
    }
}

/// The five view modes in order, with their access keys.
fn modes() -> [(String, Mode); 5] {
    [
        (tr!("menu-modes-thumbnails"), Mode::Thumbnails),
        (tr!("menu-modes-tiles"), Mode::Tiles),
        (tr!("menu-modes-icons"), Mode::Icons),
        (tr!("menu-modes-list"), Mode::List),
        (tr!("menu-modes-details"), Mode::Details),
    ]
}

/// The Details columns the Arrange Icons by submenu sorts on, and the
/// Recycle Bin's.
fn sort_columns() -> Vec<String> {
    vec![
        tr!("menu-sort-columns-name"),
        tr!("menu-sort-columns-size"),
        tr!("menu-sort-columns-type"),
        tr!("menu-sort-columns-modified"),
    ]
}

fn bin_sort_columns() -> Vec<String> {
    vec![
        tr!("menu-bin-sort-columns-name"),
        tr!("menu-bin-sort-columns-original-location"),
        tr!("menu-bin-sort-columns-date-deleted"),
        tr!("menu-bin-sort-columns-size"),
        tr!("menu-bin-sort-columns-type"),
        tr!("menu-bin-sort-columns-modified"),
    ]
}

fn new_menu(writable: bool, templates: &[reprisefs::templates::Template]) -> Menu {
    let mut m = Menu::default();
    m.row(row(tr!("menu-new-folder"), writable), Command::NewFolder);
    m.row(
        row(tr!("menu-new-shortcut"), writable),
        Command::NewShortcut,
    );
    m.sep();
    m.row(
        row(tr!("menu-new-text-document"), writable),
        Command::NewTextDocument,
    );
    for t in templates {
        let label = escape_mnemonic(&t.label);
        m.row(row(&label, writable), Command::NewFrom(t.path.clone()));
    }
    m
}

/// View > Toolbars. Links has nothing behind it and stays greyed, as do
/// the two rows below the separator.
fn toolbars_menu(state: &MenuState) -> Menu {
    let mut m = Menu::default();
    m.row(
        check(
            tr!("menu-toolbars-standard-buttons"),
            true,
            state.toolbars[0],
        ),
        Command::ToggleToolbar(0),
    );
    m.row(
        check(tr!("menu-toolbars-address-bar"), true, state.toolbars[1]),
        Command::ToggleToolbar(1),
    );
    m.row(
        check(tr!("menu-toolbars-links"), false, state.toolbars[2]),
        Command::Nothing,
    );
    m.sep();
    m.row(
        check(tr!("menu-toolbars-lock-the-toolbars"), false, false),
        Command::Nothing,
    );
    m.row(row(tr!("menu-toolbars-customize"), false), Command::Nothing);
    m
}

/// View > Explorer Bar: Folders. Favorites, History and the Tip of the
/// Day have no counterpart.
fn explorer_bar_menu(state: &MenuState) -> Menu {
    let mut m = Menu::default();
    m.row(
        keyed(tr!("menu-explorer-bar-favorites"), false, "Ctrl+I"),
        Command::Nothing,
    );
    m.row(
        row(tr!("menu-explorer-bar-history"), false),
        Command::Nothing,
    );
    m.row(
        check(tr!("menu-explorer-bar-folders"), true, state.folders),
        Command::ToggleFolders,
    );
    m.sep();
    m.row(
        row(tr!("menu-explorer-bar-tip-of-the-day"), false),
        Command::Nothing,
    );
    m
}

/// View's and the background menu's Arrange Icons by: the sort columns
/// as one radio group, then the three grid rows, drawn greyed.
fn arrange_menu(state: &MenuState) -> Menu {
    let mut m = Menu::default();
    let labels = if state.bin.is_some() {
        bin_sort_columns()
    } else {
        sort_columns()
    };
    for (i, label) in labels.iter().enumerate() {
        m.row(bullet(label, state.sort.0 == i), Command::Sort(i));
    }
    m.sep();
    m.row(
        row(tr!("menu-arrange-show-in-groups"), false),
        Command::Nothing,
    );
    m.row(
        check(tr!("menu-arrange-auto-arrange"), false, false),
        Command::Nothing,
    );
    m.row(
        row(tr!("menu-arrange-align-to-grid"), false),
        Command::Nothing,
    );
    m
}

fn go_to_menu(state: &MenuState) -> Menu {
    let mut m = Menu::default();
    m.row(
        keyed(tr!("menu-go-to-back"), state.can_back, "Alt+Left"),
        Command::Back,
    );
    m.row(
        keyed(tr!("menu-go-to-forward"), state.can_forward, "Alt+Right"),
        Command::Forward,
    );
    m.row(row(tr!("menu-go-to-up-one-level"), true), Command::Up);
    m.sep();
    m.row(row(tr!("menu-go-to-home-page"), false), Command::Nothing);
    m.sep();
    m.row(
        row(tr!("menu-go-to-documents"), true),
        Command::GoTo(Place::Documents),
    );
    m.row(
        row(tr!("menu-go-to-home"), true),
        Command::GoTo(Place::Home),
    );
    m.row(
        row(tr!("menu-go-to-computer"), true),
        Command::GoTo(Place::Computer),
    );
    m.row(
        row(tr!("menu-go-to-network"), true),
        Command::GoTo(Place::network()),
    );
    m
}

/// The five modes as one radio group: the View menu's own rows are flat,
/// the background menu's View submenu and the Views drop are this.
fn modes_menu(mode: Mode) -> Menu {
    let mut m = Menu::default();
    for (label, want) in modes() {
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
    m.row(
        row(tr!("menu-open-with-choose-program"), true),
        Command::ChooseProgram,
    );
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

fn eject_row(m: &mut Menu, state: &MenuState) {
    let label = if state.share {
        tr!("menu-eject-row-disconnect")
    } else {
        tr!("menu-eject-row-eject")
    };
    m.row(row(label, true), Command::Eject);
}

/// The six menus of the bar with their commands, greyed and checked for
/// `state`. [`menu_bar`] draws them and [`command_for`] resolves a row.
fn bar_menus(state: &MenuState) -> Vec<(String, Menu)> {
    let sel = state.selection > 0;
    let files = state.selection > state.shell_rows;
    let writable = files && !state.read_only;

    let mut file = Menu::default();
    match state.bin {
        Some(_) if sel => file.row(row(tr!("menu-bar-restore"), true), Command::Restore),
        Some(n) => file.row(
            row(tr!("menu-bar-empty-recycle-bin"), n > 0),
            Command::EmptyRecycleBin,
        ),
        None => file.sub(
            row(tr!("menu-bar-new"), !state.read_only),
            new_menu(!state.read_only, &state.templates),
        ),
    }
    file.sep();
    if state.eject {
        eject_row(&mut file, state);
        file.sep();
    }
    file.row(
        row(tr!("menu-bar-create-shortcut"), writable),
        Command::CreateShortcut,
    );
    // An item in the Recycle Bin can only be deleted for good.
    let (delete, erase) = match state.bin {
        Some(_) => (sel, true),
        None => (writable, false),
    };
    file.row(
        row(tr!("menu-bar-delete"), delete),
        Command::Delete { erase },
    );
    file.row(
        row(tr!("menu-bar-rename"), writable || state.relabel),
        Command::Rename,
    );
    // With nothing selected, Properties is the place's own sheet, which
    // Computer does not have (`props_host::place_sheet`); the Recycle Bin's
    // is Recycle Bin Properties.
    file.row(
        row(
            tr!("menu-bar-properties"),
            !state.sheetless && (sel || !state.read_only || state.bin.is_some()),
        ),
        Command::Properties,
    );
    file.sep();
    file.row(
        row(tr!("menu-bar-connect-to-server"), state.can_connect),
        Command::ConnectToServer,
    );
    file.sep();
    file.row(row(tr!("menu-bar-close"), true), Command::Close);

    let mut edit = Menu::default();
    edit.row(undo_row(state), Command::Undo);
    edit.sep();
    edit.row(keyed(tr!("menu-bar-cut"), writable, "Ctrl+X"), Command::Cut);
    // An item in the Recycle Bin is not copied out; Restore brings it
    // back.
    let copyable = files && state.bin.is_none();
    edit.row(
        keyed(tr!("menu-bar-copy"), copyable, "Ctrl+C"),
        Command::Copy,
    );
    let pasteable = (state.can_paste || state.can_paste_image) && !state.read_only;
    edit.row(
        keyed(tr!("menu-bar-paste"), pasteable, "Ctrl+V"),
        Command::Paste,
    );
    edit.row(
        row(
            tr!("menu-bar-paste-shortcut"),
            state.can_paste && !state.read_only,
        ),
        Command::PasteShortcut,
    );
    edit.sep();
    edit.row(
        row(tr!("menu-bar-copy-to-folder"), copyable),
        Command::CopyTo,
    );
    edit.row(
        row(tr!("menu-bar-move-to-folder"), writable),
        Command::MoveTo,
    );
    edit.sep();
    edit.row(
        keyed(tr!("menu-bar-select-all"), true, "Ctrl+A"),
        Command::SelectAll,
    );
    edit.row(
        row(tr!("menu-bar-invert-selection"), true),
        Command::InvertSelection,
    );

    let mut view = Menu::default();
    view.sub(row(tr!("menu-bar-toolbars"), true), toolbars_menu(state));
    view.row(
        check(tr!("menu-bar-status-bar"), true, state.status_bar),
        Command::ToggleStatusBar,
    );
    view.sub(
        row(tr!("menu-bar-explorer-bar"), true),
        explorer_bar_menu(state),
    );
    view.sep();
    for (label, mode) in modes() {
        view.row(bullet(label, state.mode == mode), Command::Mode(mode));
    }
    view.sep();
    view.sub(
        row(tr!("menu-bar-arrange-icons-by"), true),
        arrange_menu(state),
    );
    view.row(
        checked(
            keyed(tr!("menu-bar-hidden-items"), true, "Ctrl+H"),
            state.hidden,
        ),
        Command::ToggleHidden,
    );
    view.sep();
    view.row(row(tr!("menu-bar-choose-details"), false), Command::Nothing);
    view.sub(row(tr!("menu-bar-go-to"), true), go_to_menu(state));
    view.row(row(tr!("menu-bar-refresh"), true), Command::Refresh);

    let mut favorites = Menu::default();
    favorites.row(
        row(tr!("menu-bar-add-to-favorites"), state.can_add_favorite),
        Command::AddFavorite,
    );
    favorites.row(
        row(tr!("menu-bar-organize-favorites"), true),
        Command::OrganizeFavorites,
    );
    if !state.favorites.is_empty() {
        favorites.sep();
    }
    for f in &state.favorites {
        let item = MenuItem::entry(f.label.as_str()).enabled(f.go.is_some());
        favorites.row(item, f.go.clone().unwrap_or(Command::Nothing));
    }

    let mut tools = Menu::default();
    tools.row(row(tr!("menu-bar-synchronize"), false), Command::Nothing);
    tools.sep();
    tools.row(
        row(tr!("menu-bar-folder-options"), true),
        Command::FolderOptions,
    );

    let mut help = Menu::default();
    // There is no help viewer: the row is drawn and does nothing.
    help.row(
        row(tr!("menu-bar-help-and-support"), true),
        Command::Nothing,
    );
    help.sep();
    help.row(row(tr!("menu-bar-about-explorer"), true), Command::About);

    vec![
        (tr!("menu-bar-file"), file),
        (tr!("menu-bar-edit"), edit),
        (tr!("menu-bar-view"), view),
        (tr!("menu-bar-favorites"), favorites),
        (tr!("menu-bar-tools"), tools),
        (tr!("menu-bar-help"), help),
    ]
}

pub fn menu_bar(state: &MenuState) -> Bar {
    let mut bar = Bar::with_trees(
        bar_menus(state)
            .iter()
            .map(|(title, menu)| (title.as_str(), menu.tree()))
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
/// of Open. "Run as..." sits greyed under them for every file. A folder has Open in Terminal under
/// Explore, then Pin to Places (or Unpin, `state.pinned`) and Disk Usage, and Paste under Copy,
/// into it. A launcher has no Open With and has Paste under Copy, opening the clipboard's files
/// in its program. Eject sits in a group above Cut and Copy.
pub fn item_menu(
    state: &MenuState,
    kind: ItemKind,
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
        ItemKind::File if state.disk_image => {
            m.row(bold(tr!("menu-item-mount"), true), Command::MountImage);
            m.row(row(tr!("menu-item-open"), true), Command::Open);
        }
        _ => m.row(bold(tr!("menu-item-open"), true), Command::Open),
    }
    if kind == ItemKind::Folder {
        m.row(
            row(tr!("menu-item-open-in-new-tab"), true),
            Command::OpenInNewTab,
        );
    }
    if kind == ItemKind::Folder {
        m.row(row(tr!("menu-item-explore"), true), Command::Explore);
        if writable {
            m.row(row(terminal(), true), Command::OpenTerminal);
        }
        pin_row(&mut m, state.pinned, Command::Pin);
        m.row(row(tr!("menu-item-disk-usage"), true), Command::DiskUsage);
        m.row(
            row(tr!("menu-item-sharing-and-security"), false),
            Command::Nothing,
        );
    } else {
        m.row(row(tr!("menu-item-run-as"), false), Command::Nothing);
        m.sep();
        if kind.is_file() {
            m.sub(
                row(tr!("menu-item-open-with"), true),
                open_with_menu(open_with),
            );
        }
        if let Some(to) = &state.extract {
            m.row(
                row(tr!("menu-item-extract-here"), true),
                Command::Extract { here: true },
            );
            m.row(
                row(
                    tr!("menu-item-extract-to", folder = escape_mnemonic(to)),
                    true,
                ),
                Command::Extract { here: false },
            );
        }
    }
    if let Some(zip) = &state.compress {
        let label = tr!("menu-item-compress-to", archive = escape_mnemonic(zip));
        m.row(row(label, true), Command::Compress);
    }
    m.sub(
        row(tr!("menu-item-send-to"), !send_to.is_empty() && writable),
        send_to_menu(send_to),
    );
    if state.eject {
        m.sep();
        eject_row(&mut m, state);
    }
    m.sep();
    m.row(row(tr!("menu-item-cut"), writable), Command::Cut);
    m.row(row(tr!("menu-item-copy"), true), Command::Copy);
    match kind {
        ItemKind::Folder => m.row(
            row(
                tr!("menu-item-paste"),
                (state.can_paste || state.can_paste_image) && writable,
            ),
            Command::PasteOnItem,
        ),
        ItemKind::Launcher => m.row(
            row(tr!("menu-item-paste"), state.can_paste),
            Command::PasteOnItem,
        ),
        ItemKind::File | ItemKind::Script(_) => {}
    }
    m.row(
        row(tr!("menu-item-copy-as-path"), state.local_paths),
        Command::CopyAsPath,
    );
    m.sep();
    m.row(
        row(tr!("menu-item-create-shortcut"), writable),
        Command::CreateShortcut,
    );
    m.row(
        row(tr!("menu-item-delete"), writable),
        Command::Delete { erase: false },
    );
    m.row(
        row(tr!("menu-item-rename"), writable || state.relabel),
        Command::Rename,
    );
    m.sep();
    m.row(row(tr!("menu-item-properties"), true), Command::Properties);
    m
}

/// Whether Properties on a Desktop row for `place` shows a sheet: Computer and Network have none.
pub fn row_has_sheet(place: &Place) -> bool {
    matches!(place, Place::Documents | Place::Home | Place::RecycleBin)
}

/// The right-click menu of a Desktop row standing for `place`: it opens, and Properties shows the
/// place's own sheet where it has one, as the Folders tree's menu does.
pub fn shell_row_menu(place: &Place) -> Menu {
    let sheet = row_has_sheet(place);
    let mut m = Menu::default();
    m.row(bold(tr!("menu-shell-row-open"), true), Command::Open);
    m.row(
        row(tr!("menu-shell-row-open-in-new-tab"), true),
        Command::OpenInNewTab,
    );
    m.row(row(tr!("menu-shell-row-explore"), true), Command::Explore);
    m.sep();
    m.row(
        row(tr!("menu-shell-row-properties"), sheet),
        Command::Properties,
    );
    m
}

/// The right-click menu of an item in the Recycle Bin: Restore, Cut and Copy as Path (greyed),
/// Delete for good, then Properties as the default row.
fn bin_item_menu() -> Menu {
    let mut m = Menu::default();
    m.row(row(tr!("menu-bin-item-restore"), true), Command::Restore);
    m.sep();
    m.row(row(tr!("menu-bin-item-cut"), false), Command::Nothing);
    m.row(
        row(tr!("menu-bin-item-copy-as-path"), false),
        Command::Nothing,
    );
    m.sep();
    m.row(
        row(tr!("menu-bin-item-delete"), true),
        Command::Delete { erase: true },
    );
    m.sep();
    m.row(
        bold(tr!("menu-bin-item-properties"), true),
        Command::Properties,
    );
    m
}

pub fn background_menu(state: &MenuState) -> Menu {
    let writable = !state.read_only;
    let pasteable = (state.can_paste || state.can_paste_image) && writable;
    let mut m = Menu::default();
    m.sub(
        row(tr!("menu-background-view"), true),
        modes_menu(state.mode),
    );
    m.sep();
    m.sub(
        row(tr!("menu-background-arrange-icons-by"), true),
        arrange_menu(state),
    );
    m.row(row(tr!("menu-background-refresh"), true), Command::Refresh);
    if writable {
        m.row(row(terminal(), true), Command::OpenTerminalHere);
    }
    m.sep();
    m.row(
        row(tr!("menu-background-customize-this-folder"), false),
        Command::Nothing,
    );
    m.sep();
    m.row(row(tr!("menu-background-paste"), pasteable), Command::Paste);
    m.row(
        row(
            tr!("menu-background-paste-shortcut"),
            state.can_paste && writable,
        ),
        Command::PasteShortcut,
    );
    m.row(undo_row(state), Command::Undo);
    m.sep();
    m.sub(
        row(tr!("menu-background-new"), writable),
        new_menu(writable, &state.templates),
    );
    m.sep();
    // The background's Properties is the place's own sheet; the places with
    // no folder behind them have none, but for the Recycle Bin's.
    m.row(
        row(
            tr!("menu-background-properties"),
            !state.sheetless && (writable || state.bin.is_some()),
        ),
        Command::Properties,
    );
    m
}

/// Pin to Places, or Unpin from Places when `pinned`, each running `cmd` with whether it pins;
/// no row with `pinned` None.
fn pin_row(m: &mut Menu, pinned: Option<bool>, cmd: fn(bool) -> Command) {
    match pinned {
        Some(false) => m.row(row(tr!("menu-pin-row-pin-to-places"), true), cmd(true)),
        Some(true) => m.row(row(tr!("menu-pin-row-unpin-from-places"), true), cmd(false)),
        None => {}
    }
}

/// The right-click menu of a row in the Folders tree. Its first row is the
/// node's default action: Collapse, Expand, or greyed when nothing is to show. A node with a
/// folder behind it pins it as `pinned` says, as [`item_menu`] does. Properties is live for a
/// plain folder and, with `bin`, for the Recycle Bin's own sheet.
pub fn tree_menu(
    expanded: bool,
    has_children: bool,
    folder: bool,
    bin: bool,
    pinned: Option<bool>,
) -> Menu {
    let mut m = Menu::default();
    let label = if expanded {
        tr!("menu-tree-collapse")
    } else {
        tr!("menu-tree-expand")
    };
    m.row(bold(label, has_children), Command::ToggleNode);
    m.row(row(tr!("menu-tree-explore"), true), Command::OpenNode);
    m.row(row(tr!("menu-tree-open"), true), Command::OpenNode);
    pin_row(&mut m, pinned, Command::NodePin);
    m.sep();
    m.row(
        row(tr!("menu-tree-sharing-and-security"), false),
        Command::Nothing,
    );
    m.sub(row(tr!("menu-tree-send-to"), false), send_to_menu(&[]));
    m.sep();
    // The file rows act on the node itself, so they are live only for a
    // plain folder; a shell place or a drive is not a file to cut.
    m.row(row(tr!("menu-tree-cut"), folder), Command::NodeCut);
    m.row(row(tr!("menu-tree-copy"), folder), Command::NodeCopy);
    m.sep();
    m.row(row(tr!("menu-tree-delete"), folder), Command::NodeDelete);
    // Rename needs the tree's own label editor, which nothing draws yet.
    m.row(row(tr!("menu-tree-rename"), false), Command::Nothing);
    m.sep();
    m.row(
        row(tr!("menu-tree-properties"), folder || bin),
        Command::NodeProperties,
    );
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
        item(&tr!("menu-drop-copy-here"), DropOp::Copy, source.copy),
        Command::Drop(DropOp::Copy),
    );
    m.row(
        item(&tr!("menu-drop-move-here"), DropOp::Move, source.move_),
        Command::Drop(DropOp::Move),
    );
    m.row(
        item(
            &tr!("menu-drop-create-shortcuts-here"),
            DropOp::Shortcut,
            source.copy || source.move_,
        ),
        Command::Drop(DropOp::Shortcut),
    );
    m.sep();
    m.row(row(tr!("menu-drop-cancel"), true), Command::DropCancel);
    m
}

#[cfg(test)]
#[path = "menus_tests.rs"]
mod tests;
