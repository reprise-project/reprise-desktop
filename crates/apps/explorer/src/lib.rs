//! Explorer: the file manager. The window is glue: the chrome is
//! reprisekit's controls, the file system is reprisefs, and every input
//! resolves to a [`commands::Command`] before anything changes.

#[cfg(test)]
reprisebus::test_session!();

reprisekit::strings!("explorer");
use reprisekit::tr;

mod acting;
mod bin_props;
mod browse;
mod checksums;
mod chrome;
mod commands;
mod copydlg;
mod details;
mod dialogs;
mod drag;
mod extracting;
mod favorites;
mod filemanager;
mod fileops;
mod folder_opts;
mod foldertree;
#[cfg(test)]
mod i18n_tests;
mod icons;
mod intozip;
mod keys;
mod live;
mod menus;
mod navrow;
mod newshortcut;
mod openwith;
mod pane;
mod place;
mod pointer;
mod preview;
mod props_dialog;
mod props_host;
mod props_picture;
mod removing;
mod rename;
mod search;
mod sendreads;
mod settings;
mod shares;
mod space;
mod tabs;
mod tree;
mod volumes;
mod window;
mod zipkey;

#[cfg(test)]
use dialogs::DIALOG;
use dialogs::{Dialog, PROGRAM_BROWSE, is_dialog};
use menus::MenuState;
use place::{Found, Listed, Place, archive_place, list, sort_order_grouped};
use props_dialog::{PROPS, Props};
use repriseapp::decode::Decoders;
use repriseapp::keys::AltArm;
use repriseapp::listicons::{ListIcons, ReadIcons};
use repriseapp::listjob::Job;
use repriseapp::thumbs::{self, Thumbs};
use repriseapp::{
    App, BTN_LEFT, BTN_RIGHT, Child, Ctx, DropForm, Event, FileDrop, MAIN, PointerInput, Reply,
    WinId,
};
use reprisebus::instance::Handoff;
use reprisefs::folderwatch::Watch;
use reprisefs::history::History;
use reprisefs::listing::Item;
use reprisefs::netshare::Address;
use reprisefs::ops::DropOp;
use reprisefs::program::{Confirm, ScriptOpen};
use reprisekit::controls::addressbar::AddressBar;
use reprisekit::controls::combo::{self, ListItem};
use reprisekit::controls::explorerbar::ExplorerBar;
use reprisekit::controls::hosted::Hosted;
use reprisekit::controls::menubar::{self, Bar, Key};
use reprisekit::controls::pathcomplete;
use reprisekit::controls::scrollbar::ThumbDrag;
use reprisekit::controls::searchfield::SearchField;
use reprisekit::controls::shellview::{self, Entry, Field, Group, LayoutCache, ShellView};
use reprisekit::controls::statusbar::{self, StatusBar};
use reprisekit::controls::taskpane::TaskPane;
use reprisekit::controls::toolbar::ToolBar;
use reprisekit::controls::treeview::{Style, TreeView};
use reprisekit::icontheme::ListingIcons;
use reprisekit::look::widgets::item;
use reprisekit::pointer::DoubleClick;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::textedit::keysym;
use reprisekit::theme::{TextUnits, Theme};
use settings::Settings;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use window::Popup;

/// Index of the Folders button in [`Explorer::toolbar`]'s items, for the latched state.
const FOLDERS_ITEM: usize = 4;
/// How far the pointer must travel from a press before it is a drag, not a click.
const DRAG_SLOP: i32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Address,
    Search,
    Tree,
    /// The task pane, on [`TaskPane::focus`]'s stop.
    Tasks,
    /// The shell layout's command bar, on its `focus` stop.
    Commands,
    View,
}

pub type Hover = repriseapp::tip::Rest<HoverOn>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoverOn {
    Entry(usize),
    Pane(usize, usize),
}

/// Where its top-left goes and what it says. An entry's is the infotip's box; a pane row's the plain tooltip's.
pub struct Tip {
    pub at: (i32, i32),
    pub lines: Vec<String>,
    pub plain: bool,
}

/// A start that waits on its token, asked for with [`Reply::handoffs`].
pub(crate) enum Launch {
    /// The configured terminal's command line, in this folder.
    Terminal(Vec<std::ffi::OsString>, PathBuf),
    /// The program file at this path, run as this command line in its folder and app scope.
    Run(
        PathBuf,
        Vec<std::ffi::OsString>,
        Option<reprisefs::apps::AppId>,
    ),
}

/// Where [`Explorer::open_folder`] opens a folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Follow {
    Here,
    NewTab,
    /// A window of its own (`[folders] new_window`); a place with no folder opens here.
    NewWindow,
}

/// Starts `argv` as its own process in app `app`'s scope, its window taking the focus.
pub(crate) fn spawn_in(argv: &[&std::ffi::OsStr], app: &'static str) -> Reply {
    Reply {
        spawn: vec![argv.iter().map(|a| a.to_os_string()).collect()],
        spawn_app: Some(reprisefs::apps::AppId::from_static(app)),
        ..Reply::none()
    }
}

/// What opening a file came to: a program run, or the opening of a file in its program.
pub(crate) enum Opened {
    /// A program file to run as this command line in this app scope, once its token is in
    /// ([`Launch::Run`]).
    Run(Vec<std::ffi::OsString>, Option<reprisefs::apps::AppId>),
    Ran(std::io::Result<()>),
    /// A file `confirm_scripts` asks about first, as [`Asked`] says; a script runs as the
    /// [`ScriptOpen`] says once Run is picked.
    Ask(ScriptOpen, reprisefs::program::Asked),
    Open(Result<reprisefs::props::Opening, reprisefs::apps::ExecRefused>),
    /// A file whose type has no program: Open With asks for one.
    Choose,
    /// An archive that opens as a compressed folder ([`reprisefs::archive::opens_as_folder`]).
    Browse,
    /// A file of a compressed folder copied out to `.0`, and what opening the copy came to.
    Copied(PathBuf, Box<Opened>),
    /// A file of a compressed folder that could not be copied out, and why.
    NotCopied(String),
    /// A file of a compressed folder that would run as a program, which is not opened.
    Runs,
}

/// How long a listing is waited for before the window goes on. Tests wait as long as it takes.
pub(crate) const LIST_WAIT: std::time::Duration = if cfg!(test) {
    std::time::Duration::from_secs(60)
} else {
    repriseapp::listjob::WAIT
};

/// A listing read still in flight. A relist with the same key waits on it instead of starting
/// another, so relists of a folder on a dead mount hold one listing thread.
pub(crate) struct Reading {
    /// The place, hidden-files setting and icon scale the read was started with.
    key: (Place, bool, i32),
    job: Job<Read>,
    /// A relist joined the read after it started: the folder is read once more when it lands.
    again: bool,
}

/// Typed address text ([`Explorer::navigate`]) or the command line's place waiting on whether its
/// folder exists.
pub(crate) struct Checking {
    /// None for the command line's place ([`Explorer::open`]).
    typed: Option<String>,
    place: Place,
    /// The place shown at Enter: an answer landing after the window moved on is dropped.
    from: Place,
    /// What the text names once checked.
    job: Job<Found>,
}

/// A place as a listing's thread read it ([`Explorer::show`]), with the children of the
/// Folders nodes the walk to it fills.
pub(crate) struct Read {
    place: Place,
    listed: Listed,
    icons: ReadIcons,
    /// The sizes the icon files were decoded at ([`shellview::listing_px`]).
    sizes: [i32; 3],
    tree: Vec<(Place, foldertree::Kids)>,
    /// The Templates folder's files, read with a folder's listing; None for search results.
    templates: Option<Vec<reprisefs::templates::Template>>,
}

pub struct Explorer {
    pub place: Place,
    pub history: History<Place>,
    pub sv: ShellView,
    sv_layout: LayoutCache,
    pub items: Vec<Item>,
    /// By path, for the thumbnails the worker hands back. Rebuilt by [`Explorer::resort`].
    item_index: HashMap<PathBuf, usize>,
    pub tree: TreeView,
    pub tree_places: foldertree::Places,
    /// The Folders nodes whose children wear their own or special-folder icons, read again at a
    /// new scale.
    tree_art: Vec<Place>,
    /// The drives Computer lists. The scan stats every mount (a hung network
    /// mount blocks it), so it runs on a listing thread in [`Explorer::sync_drives`].
    pub drives: Vec<reprisefs::places::Drive>,
    /// Tells a drive's root apart from a plain folder in the tree.
    pub drive_mounts: Vec<PathBuf>,
    mounts_seen: String,
    /// Computer's drives' space, read at each listing ([`space`]).
    pub(crate) spaces: space::Spaces,
    pub(crate) statvfs: space::Statvfs,
    pub(crate) folder_free: space::FolderFree,
    /// A drive scan still being read, with the `/proc/mounts` it was started for.
    pub(crate) scanning: Option<(String, Job<Vec<reprisefs::places::Drive>>)>,
    pub(crate) udisks: Option<reprisefs::udisks::Udisks>,
    /// The drive the Rename box is up on, as it was when it opened.
    pub(crate) relabeling: Option<reprisefs::udisks::Volume>,
    /// The balloon under the Rename box after a key the drive's name had no room for.
    pub(crate) rename_balloon: Option<String>,
    /// Read with every drive scan and every Computer listing, and taken from
    /// `volume_changes`: a stick plugged in without being mounted doesn't change `/proc/mounts`.
    pub(crate) volumes: reprisefs::udisks::Snapshot,
    /// [`reprisefs::udisks::follow`]'s snapshots, each with a wake.
    volume_changes: Option<std::sync::mpsc::Receiver<reprisefs::udisks::Snapshot>>,
    /// The network shares gvfs has mounted, Computer's Network Locations.
    pub(crate) shares: reprisefs::netshare::Snapshot,
    /// [`reprisefs::netshare::follow`]'s snapshots, each with a wake.
    share_changes: Option<std::sync::mpsc::Receiver<reprisefs::netshare::Snapshot>>,
    /// The phones and cameras gvfs's volume monitors list, without their mounts.
    pub(crate) portable: Vec<reprisefs::portable::Device>,
    /// [`reprisefs::portable::follow`]'s lists, each with a wake.
    portable_changes: Option<std::sync::mpsc::Receiver<Vec<reprisefs::portable::Device>>>,
    /// A mount holding the place arrived after the tree followed it there: the tree follows the
    /// place again once Computer's node is read, unless a walk to the place got there first.
    pub(crate) refollow: bool,
    /// A connection to a server under way ([`shares`]).
    pub(crate) connection: Option<shares::Connection>,
    /// The address last typed into Connect to Server, offered again.
    pub(crate) last_server: String,
    /// Read once: they change on a package install, not between listings.
    pub(crate) globs: Arc<reprisefs::apps::MimeDb>,
    /// The home and document folders a listing gives their own icon, read once.
    specials: Vec<(PathBuf, &'static str)>,
    pub bar: Bar,
    pub toolbar: ToolBar,
    /// Glass's command bar, and what each of its commands does ([`chrome::commands_for`]).
    pub cmdbar: reprisekit::controls::commandbar::Commandbar,
    pub(crate) cmdbar_acts: Vec<chrome::BarAct>,
    pub address: AddressBar,
    /// The shell layout's address while it is not edited; the edit is `address`'s field.
    pub crumbs: reprisekit::controls::breadcrumb::Breadcrumb,
    /// The shell layout's Back, Forward and recent places.
    pub nav: reprisekit::controls::navbuttons::NavButtons,
    /// The shell layout's details pane, its facts set with the status bar's panels.
    pub details: reprisekit::controls::detailspane::DetailsPane,
    /// The look took the shell layout when the window last looked ([`Explorer::follow_look`]).
    shell_on: bool,
    /// The narrowest client the shell layout keeps its nav row whole in, at the last look.
    shell_min_w: i32,
    /// The folder read the address bar's completion asked for.
    address_job: repriseapp::pathjob::PathJob,
    pub address_places: Vec<Place>,
    /// A breadcrumb chevron menu's subfolder read still out ([`Explorer::open_crumb_menu`]).
    pub(crate) crumb_menu: Option<chrome::CrumbMenu>,
    /// The toolbar's Search field ([`search`]).
    pub search_field: SearchField,
    pub(crate) search: search::Search,
    /// A search still running after [`LIST_WAIT`] ([`Explorer::take_search`]).
    pub(crate) search_job: Option<Job<search::Found>>,
    /// The tool behind a search: [`reprisefs::search::try_run`], or a test's own.
    pub(crate) search_run: search::Runner,
    pub explorer_bar: ExplorerBar,
    pub taskpane: TaskPane,
    pub pane_cmds: Vec<Vec<commands::Command>>,
    pub pane_tips: Vec<Vec<Option<String>>>,
    /// What the task sections were built for, so a selection change that keeps
    /// them only rewrites Details instead of refetching every row's icon.
    pane_key: Option<pane::PaneKey>,
    /// The icons the toolbar, address bar, status bar and task pane wear stand-ins for
    /// ([`Explorer::rewear_chrome`]).
    chrome_waits: repriseapp::iconjobs::IconWaits,
    /// The Favorites menu's folder and server icons, at the menu's icon size.
    favorite_icons: [Option<Canvas>; 2],
    /// The special folders' icons by name, for their Favorites rows.
    special_icons: Vec<(&'static str, Option<Canvas>)>,
    pub status: StatusBar,
    pub settings: Settings,
    /// `settings` changed since the last write. A resize delivers a burst of
    /// events; the file is written on the next blink, on losing focus or on closing.
    settings_dirty: bool,
    /// The main window's client size at its last unmaximized resize when the user sized it, kept
    /// in 100% px at the next save. `None` while it is at its opening size, held or not: the saved
    /// size stands.
    window_px: Option<(i32, i32)>,
    pub popup: Option<Popup>,
    pub focus: Focus,
    /// The Undo rows' verb, None while greyed.
    pub undo: Option<&'static str>,
    /// The entry the Undo rows name, which they and Ctrl+Z take.
    pub(crate) undo_entry: Option<reprisefs::undo::EntryId>,
    /// An undo runs in some window ([`reprisefs::undo::Hold::Undo`]).
    pub(crate) undo_held: bool,
    pub undos: reprisefs::undo::SharedUndo,
    /// [`Explorer::edit_undos`]'s thread, and its edits whose results have yet to run.
    undo_edits: Option<std::sync::mpsc::Sender<fileops::UndoEdit>>,
    pub(crate) undo_edits_pending: usize,
    /// New's template rows, as the last folder listing read them.
    templates: Vec<reprisefs::templates::Template>,
    /// Where an asked-for paste goes, captured then: the answer arrives an
    /// event later, when the window may have moved somewhere that takes no writes.
    pub paste: Option<fileops::PasteTo>,
    /// The folder or launcher the item menu was opened on, for its Paste.
    pub menu_paste: Option<fileops::PasteTo>,
    /// Where an asked-for text paste goes ([`Explorer::paste_text`]).
    paste_field: Option<rename::PasteField>,
    /// Rename boxes opened so far.
    renames: u64,
    /// Times the Address field went back to the place's text ([`Explorer::reset_address`]).
    address_resets: u64,
    /// Scanned once; Open, Open With and the chooser read it.
    pub app_index: AppIndex,
    /// Indices into `app_index`, parallel to the Open With and Send To rows.
    pub open_with_apps: Vec<usize>,
    pub open_with_path: Option<PathBuf>,
    pub send_to: Vec<reprisefs::sendto::SendToRow>,
    /// What the Send To rows are read from, off the loop.
    pub(crate) send: sendreads::SendReads,
    /// The boxes: the one up and those waiting for it ([`Explorer::raise`]).
    pub(crate) dialogs: repriseapp::asks::Queue<Dialog>,
    /// The handler running took the box up to answer it, and nothing has taken its place yet
    /// ([`Explorer::take_up`]).
    answered: bool,
    /// The window of the last box taken while `answered`, which a different box raised in its
    /// place does not reuse.
    answered_in: Option<WinId>,
    /// Swaps the box windows [`repriseapp::asks::Queue::turn`] picks ([`Explorer::dialog_win`]).
    flip: bool,
    /// The Properties sheet, in its own modeless window: it and a box can be up at once.
    pub props: Option<Props>,
    /// Recycle Bin Properties, in its own modeless window.
    pub(crate) bin_props: Option<bin_props::BinProps>,
    bin_saves: bin_props::Saves,
    /// Folder Options, in its own modal window.
    pub(crate) folder_opts: Option<folder_opts::FolderOpts>,
    folder_saves: bin_props::Saves,
    /// What `config.toml` asks of every window, read again when the file changes.
    pub(crate) folders: settings::Folders,
    config_watch: Option<repriseconf::ConfigWatch>,
    /// The `config.toml` Folder Options and Hidden Items write; None writes nothing (tests).
    pub(crate) config_file: Option<PathBuf>,
    /// A single-click press on this entry opens it once the button comes up without a drag.
    pub(crate) click_open: Option<usize>,
    /// A sheet read and waiting for its icons to land before it goes up; for the open sheet read
    /// again after its Apply, that sheet's [`dialogs::SheetId::opened`].
    props_due: Option<(
        props_host::SheetRead,
        repriseapp::iconjobs::IconWaits,
        Option<u64>,
    )>,
    /// How many sheets have opened ([`Explorer::next_sheet`]).
    sheets_opened: u64,
    /// Space's preview of the selected item, in its own modeless window.
    pub(crate) preview: Option<preview::Preview>,
    /// The main window's client size at its last draw: the preview's arrows lay the view out by it.
    main_size: (i32, i32),
    /// The message boxes' icon, worn once it lands.
    error_icon: Option<icons::IconSlot>,
    /// `children(&self)` has no `Ctx` to borrow a renderer from.
    dialog_text: Rc<TextRenderer>,
    /// The dialog font's px at 100%; the folder picker and the Open With chooser stay at it.
    dialog_px: u32,
    /// The forms theme with the dialog font at the text size, which the boxes draw with and
    /// size their windows by ([`Explorer::follow_forms`]).
    forms: Theme,
    clicks: DoubleClick<usize>,
    /// For the double click in the rename box that selects a word.
    rename_clicks: DoubleClick<()>,
    /// A left press in the rename box holds the pointer: motion extends its selection.
    rename_drag: bool,
    /// For the double click that takes a list row as the default button, keyed by the form item.
    dialog_clicks: DoubleClick<Option<usize>>,
    /// For the double click on a Details column divider that fits the column.
    divider_clicks: DoubleClick<usize>,
    alt_arm: AltArm,
    /// The main window holds the keyboard, so a focused field's caret shows and blinks.
    keyboard: bool,
    /// The main window has held the keyboard once ([`Explorer::keyboard_entered`]).
    entered: bool,
    /// The zip whose password box waits for the window's first hold of the keyboard.
    key_wait: Option<PathBuf>,
    /// The last input was a key, not a pointer press: a dialog opened now shows keyboard cues from the start.
    keyed: bool,
    /// The held splitter's grab: the press's x less the bar's right edge.
    dragging: Option<i32>,
    /// A marquee drag in the view; its rectangle lives in `sv.marquee`.
    marquee_from: Option<shellview::MarqueeDrag>,
    /// A Details column divider drag: the column, the press position and the column's 100% width then.
    resizing: Option<(usize, i32, i32)>,
    thumb: ThumbDrag,
    tree_thumb: ThumbDrag,
    pub drag: Option<drag::Drag>,
    pub incoming: Option<drag::Incoming>,
    pub pending_drop: Option<drag::PendingDrop>,
    /// When a drop's put started, until it asks nothing more: a box it raises within
    /// [`dialogs::DROP_RAISE`] asks for the keyboard, which the compositor grants a drop target.
    pub(crate) dropped_at: Option<std::time::Instant>,
    /// What the drag's files are to each folder it rested on ([`drag::Facts`]).
    pub(crate) drop_facts: Vec<drag::FactsRead>,
    /// The status bar's text for a selected shortcut, read late or still being read.
    pub(crate) link_status: Option<chrome::LinkStatus>,
    pub(crate) put: Option<fileops::Put>,
    /// Puts that gave up with their stuck call still out ([`reprisefs::jobs::PutJob::settled`]).
    pub(crate) unsettled: Vec<reprisefs::jobs::PutJob>,
    pub(crate) put_queue: std::collections::VecDeque<fileops::PendingPut>,
    /// Batches being checked on a listing thread ([`fileops::Checked`]: a folder merge walks the
    /// folder), oldest first.
    pub(crate) clashing: std::collections::VecDeque<fileops::Clashing>,
    pub(crate) removing: Option<removing::Removing>,
    pub(crate) remove_queue: std::collections::VecDeque<removing::PendingRemoval>,
    pub(crate) extracting: Option<extracting::Extracting>,
    /// Extractions and compressions asked for while one runs, oldest first.
    pub(crate) extract_queue: std::collections::VecDeque<extracting::Batch>,
    /// The whole percent and paused state last put on the task button.
    pub(crate) task_progress: Option<(u32, bool)>,
    /// The reason the process last held off log off with ([`Explorer::hold_logout`]).
    pub(crate) logout_held: Option<String>,
    /// Finished background jobs (deletes, the bin's Empty) waiting for a wake.
    jobs_tx: std::sync::mpsc::Sender<fileops::JobDone>,
    jobs_rx: std::sync::mpsc::Receiver<fileops::JobDone>,
    pub(crate) jobs_pending: usize,
    /// [`Explorer::spawn_read_job`]'s reads still out.
    reads: Vec<Job<fileops::JobDone>>,
    /// Actions still out after the listing wait ([`Explorer::act`]).
    pub(crate) acting: Vec<acting::Acting>,
    /// How long an action waits on the file system: [`acting::ACTION_DEADLINE`], shorter in tests.
    pub(crate) action_deadline: std::time::Duration,
    /// Set by a change that landed past its deadline; the next wake re-lists.
    pub(crate) landed_late: Arc<std::sync::atomic::AtomicBool>,
    /// Close came while a put or job was out: the program quits at the wake that finds them all done.
    pub(crate) closing: bool,
    /// Started for one job or box from the command line ([`Start::alone`]): the folder window never shows.
    pub(crate) alone: bool,
    /// The thumbnail worker. Every listing bumps the generation, which drops
    /// work for a folder the user has left.
    thumbs: Thumbs,
    generation: u64,
    /// The compressed folders' listings, kept while their archives stay unchanged.
    pub(crate) archives: reprisefs::archive::Listings,
    /// The compressed folder shown is a zip that takes files, as its listing read
    /// ([`place::Listed::zip_writes`]).
    pub(crate) zip_writes: bool,
    /// Where the compressed folder shown extracts to, as its listing read it
    /// ([`place::Listed::extract_to`]).
    pub(crate) extract_to: Option<PathBuf>,
    /// The drags out of compressed folders whose copies are coming out or not yet taken.
    pub(crate) drag_copies: Vec<drag::DragCopy>,
    /// The most bytes a drag out of a compressed folder copies out ([`drag::DRAG_MOST`]).
    pub(crate) drag_most: u64,
    /// The copies out of compressed folders the puts read, held until no put runs
    /// ([`Explorer::release_put_holds`]).
    pub(crate) put_holds: Vec<reprisefs::archive::Scratch>,
    /// The copies a compressed folder's Copy put on the clipboard, held while they are on it and
    /// handed over when the window closes, as the clipboard history may paste them later.
    pub(crate) clip_scratch: extracting::ClipCopies,
    /// The scale the main window last drew at ([`Explorer::follow_scale`]).
    drawn_scale: i32,
    /// The size the toolbar's icons were last fetched at
    /// ([`reprisekit::controls::toolbar::big_icon`]).
    toolbar_px: i32,
    /// The icon theme the main window last drew from: a new one (a switch, or a reload after
    /// an icon install) fetches the chrome and re-lists.
    drawn_theme: Option<std::sync::Weak<reprisekit::icontheme::IconTheme>>,
    /// The listing's icons, fetched again when the main window draws at a new scale.
    list_icons: ListIcons,
    /// A place still being read after [`LIST_WAIT`]; the listing before stays until it lands
    /// ([`Explorer::take_read`]).
    reading: Option<Reading>,
    /// Reads of places left before they landed, one per key: a visit back joins its read
    /// instead of starting another, so visits to a dead folder hold one listing thread.
    left: Vec<Reading>,
    /// Folders nodes still being read after [`LIST_WAIT`] ([`Explorer::fill_node`]), each with
    /// whether a refill asked meanwhile reads it again once it lands.
    filling: Vec<(Place, Job<foldertree::Kids>, bool)>,
    /// Picker writes still on disk when the picker closed, landed for their undo entries.
    browse_left: Vec<crate::browse::Making>,
    /// Holds every [`Explorer::read_job`] read until the test drops its write lock; while set, no
    /// read is waited for ([`Explorer::list_wait`]).
    #[cfg(test)]
    pub(crate) held: Option<Arc<std::sync::RwLock<()>>>,
    /// Image sizes by path, from the thumbnail worker or a header read
    /// ([`Explorer::read_dims`]). They survive a sort and a refresh and feed the
    /// Tiles detail line, the infotip and the Details pane.
    image_dims: HashMap<PathBuf, (u32, u32)>,
    /// The listed folders' pictures ([`reprisefs::meta::folder_picture`]), by folder: Thumbnails
    /// view draws each one's thumbnail on its folder.
    pub(crate) pictures: HashMap<PathBuf, PathBuf>,
    /// [`Explorer::pictures`] the other way: the folders that wear each picture.
    picture_folders: HashMap<PathBuf, Vec<PathBuf>>,
    /// Images whose header was sent to a job in this listing, so a file that states no size is read once.
    dims_asked: HashSet<PathBuf>,
    /// Typed address text or the command line's place whose folder is still being checked.
    checking: Option<Checking>,
    /// The place shown when its folder went and the check of what is left
    /// ([`Explorer::follow_gone`]), still unanswered.
    going: Option<(Place, Job<live::Gone>)>,
    /// The place shown when a symlinked folder was opened, and its target's resolve
    /// ([`Explorer::open_folder`]), still unanswered.
    following: Option<(Place, Follow, Job<PathBuf>)>,
    /// Opened files whose opening ([`Explorer::open_file`]) has not answered.
    opening: Vec<(PathBuf, Job<Opened>)>,
    /// Starts waiting on their tokens, in the order asked ([`Explorer::launch`]).
    launches: std::collections::VecDeque<Launch>,
    /// Terminals [`Explorer::launch`] is starting, which a close waits for.
    starting: Vec<Job<()>>,
    /// The `--select` entries or a renamed item, and their folder, waiting on that folder's listing.
    selecting: Option<(Place, Vec<PathBuf>)>,
    watch: Option<Watch>,
    /// The bookmarks: the pinned folders, for the task pane's Pinned group and the item menu, and
    /// Favorites; None in tests that read none.
    pub(crate) pins: Option<reprisefs::pins::Pins>,
    /// Lists the place again once a link it listed unfollowed answers.
    relist: reprisefs::listing::Relist,
    /// Lists the folder again when gvfs metadata (a folder icon Nautilus sets) changes in it.
    gvfs: reprisefs::gvfsmeta::Changes,
    pub hover: Option<Hover>,
    pub tip: Option<Tip>,
    pub folder_tip: Option<(PathBuf, Vec<String>)>,
    pub tip_walk: Option<live::TipWalk>,
    /// The window's tabs; the open one's state is the window's own.
    pub(crate) tabs: reprisekit::controls::doctabs::Tabs<tabs::TabState>,
    pub(crate) tab_press: Option<tabs::TabPress>,
    /// The selection and scroll a tab that opened gets back once its folder is listed.
    tab_view: Option<(Place, Vec<PathBuf>, (usize, i32))>,
}

/// The installed programs, scanned the first time something reads them.
#[derive(Default)]
pub struct AppIndex(std::cell::OnceCell<Vec<reprisefs::apps::AppEntry>>);

impl From<Vec<reprisefs::apps::AppEntry>> for AppIndex {
    fn from(apps: Vec<reprisefs::apps::AppEntry>) -> AppIndex {
        AppIndex(apps.into())
    }
}

impl std::ops::Deref for AppIndex {
    type Target = Vec<reprisefs::apps::AppEntry>;

    fn deref(&self) -> &Self::Target {
        self.0
            .get_or_init(|| reprisefs::apps::scan(&reprisefs::apps::default_dirs()))
    }
}

impl std::ops::DerefMut for AppIndex {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let _ = &**self;
        self.0.get_mut().expect("scanned by deref")
    }
}

impl Explorer {
    fn new(start: Place, ctx: &mut Ctx) -> Explorer {
        let settings = Settings::load();
        let udisks = reprisefs::udisks::Udisks::system();
        let mut ex = Explorer::unopened(start.clone(), settings, AppIndex::default(), udisks, ctx);
        ex.follow_volumes(ctx.wake.clone());
        ex.follow_shares(ctx.wake.clone());
        ex.follow_portable(ctx.wake.clone());
        ex.first_show(start, ctx);
        ex
    }

    /// The window on `settings`, `app_index` and `udisks` as given, for tests that read nothing of the developer's own.
    #[cfg(test)]
    fn with_settings(
        start: Place,
        settings: Settings,
        app_index: AppIndex,
        udisks: Option<reprisefs::udisks::Udisks>,
        ctx: &mut Ctx,
    ) -> Explorer {
        let mut ex = Explorer::unopened(start.clone(), settings, app_index, udisks, ctx);
        ex.first_show(start, ctx);
        ex
    }

    /// The window's state before [`Explorer::first_show`] scans the drives and lists `start`.
    fn unopened(
        start: Place,
        settings: Settings,
        app_index: AppIndex,
        udisks: Option<reprisefs::udisks::Udisks>,
        ctx: &mut Ctx,
    ) -> Explorer {
        let (jobs_tx, jobs_rx) = std::sync::mpsc::channel();
        let thumb_setting = repriseconf::current().thumbnail_cache;
        let mut sv = ShellView {
            mode: commands::mode_from_name(&settings.mode),
            sort: Some(settings.sort),
            ..Default::default()
        };
        if !settings.columns.is_empty() {
            for (col, w) in sv.columns.iter_mut().zip(&settings.columns) {
                col.width = *w;
            }
        }
        Explorer {
            place: start.clone(),
            history: History::new(start.clone()),
            sv,
            sv_layout: LayoutCache::default(),
            items: Vec::new(),
            item_index: HashMap::new(),
            tree: TreeView::new(Vec::new(), Style::Explorer),
            tree_places: Default::default(),
            tree_art: Vec::new(),
            drive_mounts: Vec::new(),
            drives: Vec::new(),
            mounts_seen: String::new(),
            spaces: Default::default(),
            // A test's numbers are its own: the host's disks would land at any moment.
            statvfs: if cfg!(test) {
                |_| None
            } else {
                reprisefs::places::statvfs_totals
            },
            folder_free: Default::default(),
            scanning: None,
            volumes: reprisefs::udisks::Snapshot::default(),
            volume_changes: None,
            shares: Default::default(),
            share_changes: None,
            portable: Vec::new(),
            portable_changes: None,
            refollow: false,
            connection: None,
            last_server: String::new(),
            udisks,
            relabeling: None,
            rename_balloon: None,
            globs: Arc::new(reprisefs::apps::mime_globs()),
            specials: reprisefs::places::special_folders(),
            bar: menus::menu_bar(&MenuState::default()),
            toolbar: ToolBar {
                items: Vec::new(),
                hot: None,
                pressed: None,
                latched: None,
                big: true,
            },
            cmdbar: chrome::command_bar(),
            cmdbar_acts: Vec::new(),
            address: AddressBar::new(vec![ListItem::new("")], 0, None),
            crumbs: Default::default(),
            nav: Default::default(),
            details: Default::default(),
            shell_on: false,
            shell_min_w: 0,
            address_job: Default::default(),
            address_places: Vec::new(),
            crumb_menu: None,
            search_field: SearchField::default(),
            search: search::Search::default(),
            search_job: None,
            search_run: reprisefs::search::try_run,
            explorer_bar: ExplorerBar {
                title: tr!("explorer-unopened-folders"),
                width: settings.bar_width,
                hot_close: false,
                greyed: false,
            },
            taskpane: TaskPane::default(),
            pane_cmds: Vec::new(),
            pane_tips: Vec::new(),
            pane_key: None,
            chrome_waits: Default::default(),
            favorite_icons: [None, None],
            special_icons: Vec::new(),
            status: StatusBar {
                panels: Vec::new(),
                grip: true,
            },
            settings,
            settings_dirty: false,
            window_px: None,
            popup: None,
            focus: Focus::View,
            undo: None,
            undo_entry: None,
            undo_held: false,
            undos: match cfg!(test) {
                true => reprisefs::undo::SharedUndo::default(),
                false => reprisefs::undo::SharedUndo::session(),
            },
            undo_edits: None,
            undo_edits_pending: 0,
            templates: Vec::new(),
            paste: None,
            menu_paste: None,
            paste_field: None,
            renames: 0,
            address_resets: 0,
            app_index,
            open_with_apps: Vec::new(),
            open_with_path: None,
            send_to: Vec::new(),
            send: sendreads::SendReads::default(),
            dialogs: Default::default(),
            answered: false,
            answered_in: None,
            flip: false,
            props: None,
            bin_props: None,
            bin_saves: Default::default(),
            folder_opts: None,
            folder_saves: Default::default(),
            folders: settings::Folders::current(),
            config_watch: (!cfg!(test))
                .then(|| {
                    let wake = ctx.wake.clone();
                    repriseconf::watch(move || wake.wake())
                })
                .flatten(),
            click_open: None,
            config_file: (!cfg!(test)).then(repriseconf::config_path),
            props_due: None,
            sheets_opened: 0,
            preview: None,
            main_size: ctx.size,
            error_icon: None,
            dialog_text: ctx.text.clone(),
            dialog_px: Theme::default().fonts.size_px,
            forms: Theme::default(),
            clicks: DoubleClick::default(),
            rename_clicks: DoubleClick::default(),
            rename_drag: false,
            dialog_clicks: DoubleClick::default(),
            divider_clicks: DoubleClick::default(),
            alt_arm: AltArm::default(),
            keyboard: false,
            entered: false,
            key_wait: None,
            keyed: false,
            dragging: None,
            marquee_from: None,
            resizing: None,
            thumb: ThumbDrag::default(),
            tree_thumb: ThumbDrag::default(),
            drag: None,
            incoming: None,
            pending_drop: None,
            dropped_at: None,
            drop_facts: Vec::new(),
            link_status: None,
            put: None,
            unsettled: Vec::new(),
            put_queue: std::collections::VecDeque::new(),
            clashing: std::collections::VecDeque::new(),
            removing: None,
            remove_queue: std::collections::VecDeque::new(),
            extracting: None,
            extract_queue: std::collections::VecDeque::new(),
            task_progress: None,
            logout_held: None,
            jobs_tx,
            jobs_rx,
            jobs_pending: 0,
            reads: Vec::new(),
            acting: Vec::new(),
            action_deadline: acting::ACTION_DEADLINE,
            landed_late: Arc::default(),
            closing: false,
            alone: false,
            thumbs: Thumbs::new(
                Arc::new(Decoders::default_set()),
                thumbs::cache_for(thumb_setting),
                thumb_setting.external_thumbnailers,
                ctx.wake.clone(),
            ),
            generation: 0,
            archives: Default::default(),
            zip_writes: false,
            extract_to: None,
            drag_copies: Vec::new(),
            drag_most: drag::DRAG_MOST,
            put_holds: Vec::new(),
            clip_scratch: Default::default(),
            drawn_scale: 0,
            toolbar_px: 0,
            drawn_theme: None,
            list_icons: ListIcons::default(),
            reading: None,
            left: Vec::new(),
            filling: Vec::new(),
            browse_left: Vec::new(),
            #[cfg(test)]
            held: None,
            image_dims: HashMap::new(),
            pictures: HashMap::new(),
            picture_folders: HashMap::new(),
            dims_asked: HashSet::new(),
            checking: None,
            going: None,
            following: None,
            opening: Vec::new(),
            launches: Default::default(),
            starting: Vec::new(),
            selecting: None,
            pins: (!cfg!(test)).then(|| {
                let wake = ctx.wake.clone();
                reprisefs::pins::Pins::start(move || wake.wake())
            }),
            watch: match Watch::new({
                let wake = ctx.wake.clone();
                move || wake.wake()
            }) {
                Ok(w) => Some(w),
                Err(e) => {
                    log::warn!("no folder watch: {e}");
                    None
                }
            },
            relist: {
                let wake = ctx.wake.clone();
                reprisefs::listing::Relist::new(move || wake.wake())
            },
            gvfs: {
                let wake = ctx.wake.clone();
                reprisefs::gvfsmeta::Changes::new(move || wake.wake())
            },
            hover: None,
            tip: None,
            folder_tip: None,
            tip_walk: None,
            tabs: {
                let mut tabs = reprisekit::controls::doctabs::Tabs::new();
                tabs.strip.adder_menu = false;
                let label = start.title();
                tabs.open(
                    tabs::TabState::at(start, shellview::Mode::Details, None),
                    label,
                );
                tabs
            },
            tab_press: None,
            tab_view: None,
        }
    }

    /// Builds the chrome and lists `start`. The drives are scanned first, on a listing thread
    /// ([`Explorer::sync_drives`]): the address drop and a start under a drive need them.
    fn first_show(&mut self, start: Place, ctx: &mut Ctx) {
        self.follow_forms(ctx);
        self.follow_look(ctx);
        self.sync_drives(ctx);
        self.build_toolbar(ctx);
        self.build_address(ctx);
        self.build_tree(ctx);
        self.sync_bar();
        self.show(start, ctx);
    }

    /// The keyboard cannot be left in a tree that is gone.
    pub fn sync_bar(&mut self) {
        let gone = match self.focus {
            Focus::Tree if self.shell_on => !self.settings.nav_pane,
            Focus::Tree => !self.settings.folders,
            Focus::Tasks => self.settings.folders,
            Focus::Commands => !self.shell_on,
            _ => false,
        };
        if gone {
            self.focus = Focus::View;
        }
    }

    fn strip_h(&self, ctx: &Ctx) -> i32 {
        match self.tabs.list.len() {
            0 | 1 => 0,
            _ => reprisekit::controls::tabstrip::doc_height(ctx.theme.text),
        }
    }

    /// The shell layout's rects, under a look that takes it ([`window::is_shell`]).
    pub fn shell_layout(&self, ctx: &Ctx) -> Option<window::ShellLayout> {
        window::is_shell(&ctx.theme).then(|| {
            let cm = ctx.theme.controls();
            let active = self.bar.is_active();
            let (strip, bar_w) = (self.strip_h(ctx), self.explorer_bar.width);
            window::shell_layout(ctx.size, &self.settings, active, strip, bar_w, &cm)
        })
    }

    /// The menu bar shows: always under the rebar layout, under the shell layout while Alt or
    /// the Layout setting shows it.
    pub fn bar_shown(&self, ctx: &Ctx) -> bool {
        self.shell_layout(ctx).is_none_or(|sl| sl.menu.is_some())
    }

    /// A new look: one that swaps the layout closes every menu and list and takes the edit out
    /// of the fields, so nothing from the old layout's rects stays up, and puts the keyboard in
    /// the view.
    pub(crate) fn follow_look(&mut self, ctx: &Ctx) {
        let shell = window::is_shell(&ctx.theme);
        self.shell_min_w = window::shell_min_width(&ctx.theme.controls());
        self.bar.banded = !shell;
        if shell == self.shell_on {
            return;
        }
        self.shell_on = shell;
        self.popup = None;
        self.bar.close();
        self.close_drop_menu();
        self.address.complete.close();
        self.address.combo.open = None;
        self.crumbs.cancel_edit();
        self.crumbs.menu_closed();
        self.nav.menu_closed();
        self.nav.pressed = None;
        self.focus = Focus::View;
    }

    /// The nav row's held states follow the window's: the recent places drop is up while its menu
    /// is, and the address is edited while it holds the keyboard.
    pub(crate) fn sync_nav_row(&mut self) {
        if self.popup.is_none() {
            self.nav.menu_closed();
            self.crumbs.menu_closed();
        }
        match (self.focus == Focus::Address, self.crumbs.editing()) {
            (true, false) => self.crumbs.start_edit(),
            (false, true) => {
                self.crumbs.focus_lost();
            }
            _ => {}
        }
    }

    pub fn layout(&self, ctx: &Ctx) -> window::Layout {
        if let Some(sl) = self.shell_layout(ctx) {
            return window::Layout::of_shell(&sl, ctx.size, &ctx.theme.controls());
        }
        let strip = self.strip_h(ctx);
        window::layout(
            ctx.size,
            &self.settings,
            self.explorer_bar.width,
            statusbar::height(&ctx.theme.controls()),
            strip,
            &ctx.theme.controls(),
        )
    }

    /// How many rows of the window's top the frame's glass shows through: the rebar, under a
    /// look whose frame runs down under the client.
    pub fn glass_top(&self, ctx: &Ctx) -> i32 {
        if reprisekit::look::client_glass(&ctx.theme, true).is_none() {
            return 0;
        }
        let lay = self.layout(ctx);
        match lay.rebar.y == lay.client.y {
            true => lay.etch_y - lay.client.y,
            false => 0,
        }
    }

    /// Rebuilt only when the view's geometry or the look's control sizes changed.
    pub fn view_layout(&self, frame: Rect, ctx: &Ctx) -> Rc<shellview::Layout> {
        self.sv_layout.get(&self.sv, frame, &ctx.theme.controls())
    }

    pub fn bar_layout(&self, ctx: &Ctx) -> menubar::Layout {
        let menu = self.layout(ctx).menu;
        let mut lay = self
            .bar
            .layout(
                &ctx.text,
                ctx.theme.fonts.size_px,
                menu,
                &ctx.theme.controls(),
            )
            .reaching(ctx.menu_within());
        // The shell layout's bar strip fills its row, line and all.
        if window::is_shell(&ctx.theme) {
            lay.bar.height = menu.height;
        }
        lay
    }

    fn menu_state(&self) -> MenuState {
        let single = self.sv.selection.single();
        MenuState {
            mode: self.sv.mode,
            sort: self.sv.sort.unwrap_or(self.settings.sort),
            selection: self.sv.selection.len(),
            shell_rows: (self.sv.selection.set.iter())
                .filter(|&&i| self.shell_row(i).is_some())
                .count(),
            sheetless: self.sheetless(),
            can_paste: repriseapp::clipboard_has_files(),
            can_paste_image: repriseapp::clipboard_has_image(),
            can_undo: self.undo,
            folders: self.settings.folders,
            status_bar: self.settings.status_bar,
            layout: [
                self.settings.menu_bar,
                self.settings.details_pane,
                self.settings.nav_pane,
            ],
            shell: self.shell_on,
            hidden: self.folders.show_hidden,
            toolbars: [self.settings.toolbars, self.settings.address_bar, false],
            read_only: self.place.read_only(),
            bin: (self.place == Place::RecycleBin).then_some(self.items.len()),
            can_back: self.can_back(),
            can_forward: self.history.can_forward(),
            extract: None,
            archive: matches!(self.place, Place::Archive(..)),
            extract_entries: self.extract_folder().is_some(),
            local_paths: self.selected_local().is_some(),
            compress: None,
            disk_image: false,
            paste_into_zip: false,
            into_zip: self.zip_view().is_some(),
            relabel: (self.sv.selection.single()).is_some_and(|i| self.relabel_target(i).is_some()),
            pinned: None,
            eject: single.is_some_and(|i| self.ejectable(i) || self.share_selected(i)),
            share: single.is_some_and(|i| self.share_selected(i)),
            can_connect: self.shares.gio,
            templates: self.templates.clone(),
            can_add_favorite: self.favorite_dir().is_some(),
            favorites: favorites::menu_rows(&self.favorites(), &self.specials),
            favorite_icons: self.favorite_icons.clone(),
            special_icons: self.special_icons.clone(),
        }
    }

    /// Whether the selection is only Desktop rows that [`Explorer::properties`] opens no sheet for:
    /// several rows, or one whose place has none.
    fn sheetless(&self) -> bool {
        let row = self.sv.selection.single().and_then(|i| self.shell_row(i));
        !self.sv.selection.is_empty()
            && self.picked().next().is_none()
            && !row.is_some_and(|p| menus::row_has_sheet(&p))
    }

    /// The pinned folders as last read.
    pub(crate) fn pinned(&self) -> std::sync::Arc<Vec<reprisefs::pins::Pin>> {
        self.pins.as_ref().map(|p| p.current()).unwrap_or_default()
    }

    /// Pins `dir` to the places bar, or unpins it, on the pins thread that writes Favorites too, so
    /// neither edit undoes the other; a failure comes back as Favorites' does.
    pub(crate) fn set_pinned(&mut self, dir: &std::path::Path, pin: bool) {
        let dir = dir.to_path_buf();
        self.edit_favorites(move |b| if pin { b.pin(&dir) } else { b.unpin(&dir) });
    }

    pub(crate) fn flush_settings(&mut self) {
        if std::mem::take(&mut self.settings_dirty) {
            if let Some(px) = self.window_px {
                self.settings.keep_window(px, self.forms.text);
            }
            self.settings.save();
        }
    }

    fn sync_menus(&mut self) {
        if self.bar.open.is_some() {
            return;
        }
        self.sync_undo();
        let fresh = menus::menu_bar(&self.menu_state());
        self.bar.titles = fresh.titles;
        self.bar.menus = fresh.menus;
    }

    /// Go somewhere new, recorded in the history. Going where we already are
    /// is a refresh: Enter on unchanged address text re-lists the folder.
    pub fn go(&mut self, place: Place, ctx: &mut Ctx) -> Reply {
        if place == self.place {
            // Going where the window already is drops another place still being read.
            if let Some(r) = self.reading.take_if(|r| r.key.0 != place) {
                self.leave(r);
            }
            return self.refresh(ctx);
        }
        self.history.visit(place.clone());
        self.show(place, ctx)
    }

    /// Goes to typed address text. Whether a typed folder exists is asked on a listing thread
    /// through `is_dir`, so one on a dead mount never hangs the loop; an answer later than
    /// [`repriseapp::listjob::WAIT`] lands on a wake ([`Explorer::take_checked`]).
    pub(crate) fn navigate(
        &mut self,
        text: &str,
        is_dir: impl FnOnce(&Path) -> bool + Send + 'static,
        ctx: &mut Ctx,
    ) -> Reply {
        let typed = text.trim();
        self.checking = None;
        let Some((place, dir)) = Place::read_address(typed, Some(&self.place), &self.shares.shares)
        else {
            return self.not_found(typed, ctx);
        };
        let Some(dir) = dir else {
            return self.go(place, ctx);
        };
        let job = self.check_place(place.clone(), dir, is_dir, ctx);
        match job.wait(repriseapp::listjob::WAIT) {
            Some(found) => self.checked(Some(typed), place, found, ctx),
            None => {
                self.checking = Some(Checking {
                    typed: Some(typed.to_string()),
                    place,
                    from: self.place.clone(),
                    job,
                });
                Reply::none()
            }
        }
    }

    /// Goes to a typed folder whose check ([`Explorer::navigate`]) answered late, unless the
    /// window has moved on since.
    pub(crate) fn take_checked(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let found = self.checking.as_ref()?.job.take()?;
        let c = self.checking.take()?;
        (c.from == self.place).then(|| self.checked(c.typed.as_deref(), c.place, found, ctx))
    }

    fn checked(&mut self, typed: Option<&str>, place: Place, found: Found, ctx: &mut Ctx) -> Reply {
        match (typed, found) {
            (Some(_), Found::Place(found)) => self.go(found, ctx),
            (Some(typed), _) => self.not_found(typed, ctx),
            (None, Found::Place(found)) if found == place => Reply::none(),
            (None, Found::Place(found)) => {
                self.history = History::new(found.clone());
                self.show(found, ctx)
            }
            (None, found) => {
                self.history = History::new(Place::Home);
                let shown = self.show(Place::Home, ctx);
                // A start folder inside an archive says it is not there, as typing it would.
                match found {
                    Found::NotHeld(path) => shown.and(self.not_found(&path.to_string_lossy(), ctx)),
                    _ => shown,
                }
            }
        }
    }

    /// Checks on a listing thread that the folder `dir` typed for `place` is there, through
    /// `is_dir`, or else that it names a folder of a compressed folder ([`archive_place`]).
    fn check_place(
        &self,
        place: Place,
        dir: PathBuf,
        is_dir: impl FnOnce(&Path) -> bool + Send + 'static,
        ctx: &Ctx,
    ) -> Job<Found> {
        let (globs, apps) = (self.globs.clone(), self.app_index.clone());
        let archives = self.archives.clone();
        Job::spawn(Some(&ctx.wake), move || match is_dir(&dir) {
            true => Found::Place(place),
            false => archive_place(&dir, &globs, &apps, &archives),
        })
    }

    /// Opens the window built by `build` on the command line's [`Start`]. Its folder is checked
    /// on a listing thread beside the first listing, as typed text is ([`Explorer::navigate`]),
    /// so the window maps even when the folder sits on a dead mount; a missing folder opens Home.
    fn open(
        start: Start,
        is_dir: impl FnOnce(&Path) -> bool + Send + 'static,
        build: impl FnOnce(Place, &mut Ctx) -> Explorer,
        ctx: &mut Ctx,
    ) -> Explorer {
        let Start {
            place,
            dir,
            select,
            connect,
            alone,
            search,
            tabs,
        } = start;
        let mut ex = build(place.clone(), ctx);
        let check = dir.map(|d| ex.check_place(place.clone(), d, is_dir, ctx));
        if let Some(tabs) = tabs {
            ex.restore_tabs(tabs, ctx);
        }
        ex.alone = alone.is_some();
        ex.selecting = (!select.is_empty()).then(|| (place.clone(), select));
        if let Some(job) = check {
            // A listing still being read means the mount is slow: the check would be too.
            let found = match ex.reading {
                None => job.wait(repriseapp::listjob::WAIT),
                Some(_) => job.take(),
            };
            match found {
                Some(found) => {
                    let _ = ex.checked(None, place, found, ctx);
                }
                None => {
                    ex.checking = Some(Checking {
                        typed: None,
                        place: place.clone(),
                        from: place,
                        job,
                    })
                }
            }
        }
        ex.select_pending(ctx);
        if let Some(uri) = connect {
            let _ = ex.connect_to(&uri, ctx);
        }
        if search {
            let _ = ex.search_focus();
        }
        ex
    }

    /// Up and Back: `place` shown by `show`, with the folder the window leaves selected there.
    pub(crate) fn return_to(
        &mut self,
        place: Place,
        ctx: &mut Ctx,
        show: impl FnOnce(&mut Self, Place, &mut Ctx) -> Reply,
    ) -> Reply {
        let left = match (&place, &self.place) {
            (Place::Desktop, _) => self.place.row_path().or_else(|| self.place.path()),
            (_, Place::Archive(file, inner)) => Some(place::in_archive(file, inner)),
            _ => self.place.path(),
        };
        let reply = show(self, place.clone(), ctx);
        if let Some(left) = left {
            self.select_once_listed(place, vec![left], ctx);
        }
        reply
    }

    /// Selects `paths` in `place`, the first scrolled into view: now when its listing is shown,
    /// else once the listing being read lands.
    pub(crate) fn select_once_listed(&mut self, place: Place, paths: Vec<PathBuf>, ctx: &mut Ctx) {
        self.selecting = Some((place, paths));
        self.select_pending(ctx);
    }

    /// Selects what `selecting` holds (the command line's `--select` entries, a rename's or a
    /// put's result) once its folder's listing has landed.
    fn select_pending(&mut self, ctx: &mut Ctx) {
        if self.reading.is_some() {
            return;
        }
        if let Some((place, paths)) = self.selecting.take()
            && place == self.place
        {
            self.select_shown(&paths, ctx);
        }
    }

    /// Re-list the current place, keeping the selection by path and leaving the
    /// history alone: F5, View > Refresh, and Enter on the address already shown.
    pub fn refresh(&mut self, ctx: &mut Ctx) -> Reply {
        if let Some(reply) = self.search_again(ctx) {
            return reply;
        }
        if self.reading.as_ref().is_some_and(|r| r.key.0 != self.place) {
            // The place being read is newer than the listing shown.
            return Reply::none();
        }
        let here = self.place.clone();
        if here == Place::Computer {
            self.mounts_seen.clear();
        }
        self.keeping_view(ctx, |ex, ctx| ex.show(here, ctx))
    }

    /// Runs `list`, a listing of the place shown, keeping the selection, a rename in progress
    /// and the scroll position.
    fn keeping_view(
        &mut self,
        ctx: &mut Ctx,
        list: impl FnOnce(&mut Self, &mut Ctx) -> Reply,
    ) -> Reply {
        let chosen = self.selection_paths();
        // The folder watch fires refreshes unasked; they keep the scroll position.
        let (top, left) = (self.sv.top, self.sv.left);
        // A rename in progress survives the re-listing, by path: the folder watch
        // fires one the instant New > Folder writes its folder.
        let editing = self
            .sv
            .edit
            .as_ref()
            .and_then(|(i, ed)| self.items.get(*i).map(|it| (it.path.clone(), ed.clone())));
        let reply = list(self, ctx);
        self.select_paths(&chosen);
        if let Some((path, ed)) = editing
            && let Some(j) = self.index_of(&path)
        {
            self.sv.edit = Some((j, ed));
        }
        self.restore_scroll(top, left, ctx);
        self.refresh_chrome(ctx);
        reply
    }

    /// Lists `place`. Its folder is read on a thread; one that takes longer than [`LIST_WAIT`]
    /// lands later through [`Explorer::take_read`], the window showing the listing before
    /// until then. A search under way or shown ends here, and the Search field empties.
    pub fn show(&mut self, place: Place, ctx: &mut Ctx) -> Reply {
        if self.search.cancel() {
            self.search_field.clear();
        }
        self.search_job = None;
        self.sync_drives(ctx);
        let archive = |p: &Place| match p {
            Place::Archive(file, _) => Some(file.clone()),
            _ => None,
        };
        if archive(&place) != archive(&self.place) {
            // A listing of the archive left is stopped, and a read of it is not joined later.
            self.archives.cancel();
            self.left.retain(|r| archive(&r.key.0).is_none());
        }
        let key = (place, self.folders.show_hidden, ctx.icons.scale());
        if let Some(r) = self.reading.take_if(|r| r.key != key) {
            self.leave(r);
        }
        let joined = self.reading.take().or_else(|| {
            let i = self.left.iter().position(|r| r.key == key)?;
            Some(self.left.swap_remove(i))
        });
        let (job, again) = match joined {
            Some(r) => (r.job, true),
            None => (
                Job::spawn(Some(&ctx.wake), self.reader(key.0.clone(), ctx)),
                false,
            ),
        };
        match job.wait(self.list_wait()) {
            Some(read) => {
                let reply = self.show_read(read, ctx);
                if again {
                    self.read_again(ctx);
                }
                reply
            }
            None => {
                self.reading = Some(Reading { key, job, again });
                Reply::none()
            }
        }
    }

    /// Keeps `r`, a read of a place navigated away from, for a visit back to join, dropping the
    /// left reads that have landed.
    fn leave(&mut self, r: Reading) {
        self.left.retain(|l| l.job.take().is_none());
        if r.job.take().is_none() {
            self.left.push(r);
        }
    }

    /// Starts another read of the place shown without waiting for it: what a relist that joined
    /// an older read asked for.
    fn read_again(&mut self, ctx: &Ctx) {
        let place = self.place.clone();
        let job = Job::spawn(Some(&ctx.wake), self.reader(place.clone(), ctx));
        self.reading = Some(Reading {
            key: (place, self.folders.show_hidden, ctx.icons.scale()),
            job,
            again: false,
        });
    }

    /// The read of `place` for a listing's thread.
    pub(crate) fn reader(&self, place: Place, ctx: &Ctx) -> impl FnOnce() -> Read + Send + 'static {
        let (globs, show_hidden) = (self.globs.clone(), self.folders.show_hidden);
        let (drives, volumes) = (self.drives.clone(), self.volumes.clone());
        let shares = self.shares.shares.clone();
        let portable = self.portable.clone();
        let mounts = self.drive_mounts.clone();
        let to_fill = foldertree::to_fill(&self.tree, &self.tree_places, &place.in_tree(), &mounts);
        let scale = ctx.icons.scale();
        let sizes = shellview::listing_px(ctx.theme.text);
        let tree_px = self.tree_places.px;
        let specials = self.specials.clone();
        let archives = self.archives.clone();
        move || {
            let devices = place::Devices {
                drives: &drives,
                volumes: &volumes,
                shares: &shares,
                portable: &portable,
            };
            let listed = match &place {
                Place::Archive(file, inner) => {
                    place::archive_listing(&archives, (file, inner), &globs, show_hidden)
                }
                _ => list(&place, &globs, show_hidden, devices),
            };
            let icons = ReadIcons::read(&listed.items, sizes, scale);
            let inp = foldertree::Inputs {
                show_hidden,
                drives: &drives,
                volumes: &volumes,
                shares: &shares,
                drive_mounts: &mounts,
            };
            let tree = to_fill
                .into_iter()
                .map(|p| {
                    let kids = foldertree::read_kids(&p, &inp, &[], (tree_px, scale), &specials);
                    (p, kids)
                })
                .collect();
            let templates = reprisefs::templates::dir().map(|d| reprisefs::templates::list(&d));
            Read {
                place,
                listed,
                icons,
                sizes,
                tree,
                templates: Some(templates.unwrap_or_default()),
            }
        }
    }

    /// Shows a place read late ([`Explorer::show`]), keeping the view when it is the one shown.
    pub(crate) fn take_read(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let read = self.reading.as_ref()?.job.take()?;
        let again = self.reading.take().is_some_and(|r| r.again);
        let reply = if read.place == self.place {
            self.keeping_view(ctx, |ex, ctx| ex.show_read(read, ctx))
        } else {
            self.show_read(read, ctx)
        };
        if again {
            self.read_again(ctx);
        }
        self.select_pending(ctx);
        self.follow_tab_view(ctx);
        Some(reply)
    }

    fn show_read(&mut self, read: Read, ctx: &mut Ctx) -> Reply {
        let Read {
            place,
            listed:
                Listed {
                    items,
                    groups,
                    error,
                    zip_writes,
                    extract_to,
                    ..
                },
            icons: read_icons,
            sizes,
            tree,
            templates,
        } = read;
        if let Some(templates) = templates {
            self.templates = templates;
        }
        // A place, a kind of listing or a folder seen empty opens afresh; a re-listing keeps Name.
        let fresh = self.set_columns(&place) || place != self.place || self.items.is_empty();
        let is_12h = reprisefs::locale::is_12h();
        let searched = self.search.shown().is_some();
        read_icons.put(&mut ctx.icons);
        let drawn = shellview::drawn_sizes(self.sv.mode);
        let first = drawn.iter().position(|&d| d).unwrap_or(0);
        let probe = ctx.icons.probe_px();
        // `has_unread` decodes each small PNG at the size the view draws, which the fetch below
        // then reuses.
        ctx.icons.set_probe_px(sizes[first] * ctx.icons.scale());
        // Resolved up front: `item_icon_with` asks what the theme carries, and the icons below want the cache mutable.
        let names: Vec<String> = items
            .iter()
            .zip(&read_icons.own)
            .map(|(it, own)| match Place::of_row(&it.path) {
                Some(p) => p.icon_name().to_string(),
                None => {
                    reprisefs::icons::item_icon_with(it, own.as_deref(), &self.specials, &|n| {
                        ctx.icons.has_unread(n)
                    })
                }
            })
            .collect();
        // A re-listing of the same place keeps the thumbnails of unchanged images,
        // so a download finishing in a picture folder doesn't redecode the rest.
        let mut kept: HashMap<PathBuf, (u64, Option<i64>, Canvas)> = HashMap::new();
        if place == self.place {
            for (it, e) in self.items.iter().zip(self.sv.entries.iter_mut()) {
                if let Some(thumb) = e.thumb.take() {
                    kept.insert(it.path.clone(), (it.size, it.modified, thumb));
                }
            }
        }
        let pictures: HashMap<PathBuf, PathBuf> = (items.iter().zip(&read_icons.pictures))
            .filter_map(|(it, p)| Some((it.path.clone(), p.clone()?)))
            .collect();
        let shown: Vec<String> = items.iter().map(|it| self.shown_name(it)).collect();
        let mut icons = (ListingIcons::default().with_sizes(sizes))
            .drawing(drawn)
            .unread();
        self.sv.hot = None;
        self.sv.entries = items
            .iter()
            .enumerate()
            .map(|(i, it)| {
                let [icon, icon32, icon48] = icons.get(&mut ctx.icons, &names[i]);
                let mut e = Entry {
                    id: i as u64,
                    name: shown[i].clone(),
                    icon,
                    icon32,
                    icon48,
                    // Thumbnails arrive later from the `thumbs` worker, or are carried over below.
                    thumb: None,
                    is_dir: it.is_dir,
                    link: it.is_link || it.shortcut.is_some(),
                    cut: false,
                    hidden: it.hidden,
                    size: it.size,
                    kind: it.kind.clone(),
                    modified: it
                        .modified
                        .map(|s| reprisefs::listing::date_text(s, is_12h))
                        .unwrap_or_default(),
                    mtime: it.modified,
                    detail: Vec::new(),
                    folder: match &it.trashed {
                        Some(t) => t.original.parent(),
                        None => it.path.parent().filter(|_| searched),
                    }
                    .map(|d| d.display().to_string())
                    .unwrap_or_default(),
                    deleted: it
                        .trashed
                        .as_ref()
                        .and_then(|t| t.deleted)
                        .map(|s| reprisefs::listing::date_text(s, is_12h))
                        .unwrap_or_default(),
                    space: None,
                };
                e.detail = shellview::tile_detail(&e);
                e
            })
            .collect();
        self.list_icons.listed(icons, ctx.icons.scale());
        ctx.icons.set_probe_px(probe);
        let mut dims = HashMap::new();
        for (it, e) in items.iter().zip(self.sv.entries.iter_mut()) {
            let Some((size, modified, thumb)) = kept.remove(&it.path) else {
                continue;
            };
            if it.is_dir {
                let picture = pictures.get(&it.path);
                if picture.is_some() && picture == self.pictures.get(&it.path) {
                    e.thumb = Some(thumb);
                }
                continue;
            }
            let Some(&d) = self.image_dims.get(&it.path) else {
                continue;
            };
            if (size, modified) == (it.size, it.modified) {
                e.thumb = Some(thumb);
                e.detail = shellview::image_detail(d, &it.kind);
                dims.insert(it.path.clone(), d);
            }
        }
        self.image_dims = dims;
        self.picture_folders = HashMap::new();
        for (folder, picture) in &pictures {
            let folders = self.picture_folders.entry(picture.clone()).or_default();
            folders.push(folder.clone());
        }
        self.pictures = pictures;
        self.dims_asked.clear();
        self.sv.groups = groups
            .into_iter()
            .map(|(title, first)| Group { title, first })
            .collect();
        self.sv.selection.clear();
        self.sv.edit = None;
        self.sv.marquee = None;
        self.sv.top = 0;
        self.sv.left = 0;
        self.items = items;
        self.zip_writes = zip_writes;
        self.extract_to = extract_to;
        self.link_status = None;
        // A bin item's sheet goes once its item is no longer listed as it was.
        let stale_sheet =
            self.props
                .as_ref()
                .and_then(|p| p.restore.as_ref())
                .is_some_and(|item| {
                    !self.items.iter().any(|it| {
                        it.path == item.path && it.trashed.as_deref() == item.record.as_ref()
                    })
                });
        let closed = match stale_sheet {
            true => self.close_props(ctx),
            false => Reply::none(),
        };
        if fresh {
            self.fit_name(ctx);
        }
        self.place = place.clone();
        self.tabs.set_label(self.title_of(&place));
        self.resort();
        if place == Place::Computer {
            self.read_space(ctx);
        }
        self.read_folder_free(ctx);

        let icon = chrome::address_icon(
            &mut self.chrome_waits,
            &mut ctx.icons,
            &place,
            ctx.theme.text,
        );
        self.reset_address(&place.address_in(&self.shares.shares), icon);
        self.tree_follow_with(&place, tree, ctx);
        self.refresh_chrome(ctx);
        // The listing is new: every earlier thumbnail request is stale.
        self.generation += 1;
        self.thumbs.cancel_all();
        self.hide_tip();
        self.request_thumbs(ctx.icons.scale());
        let failed = match (&place, error) {
            (Place::Archive(file, _), Some(place::Unread::Locked)) => {
                self.ask_zip_key(file.clone(), ctx)
            }
            (Place::Archive(file, _), Some(place::Unread::Why(why))) => {
                self.archive_open_failed(file, why, ctx)
            }
            _ => Reply::none(),
        };
        if let Some(w) = self.watch.as_mut() {
            match place.path() {
                Some(_) if place == Place::RecycleBin => {
                    let add = w.watch_all_later();
                    drop(Job::spawn(None, move || add.run(&place::bin_watches())));
                }
                Some(dir) => {
                    let add = w.watch(&dir);
                    drop(Job::spawn(None, move || add.run()));
                }
                None => w.clear(),
            }
        }
        Reply {
            redraw: true,
            ..closed.and(failed)
        }
    }

    /// Details' columns for the Recycle Bin (with Original Location and Date
    /// Deleted) or for a folder (the user's widths), switched only when the kind
    /// of listing changes, which returns `true`. The bin keeps the mode and sorts by name without
    /// touching the saved sort.
    fn set_columns(&mut self, place: &Place) -> bool {
        let bin = *place == Place::RecycleBin;
        let computer = *place == Place::Computer;
        let searched = self.search.shown().is_some();
        let want = if bin {
            shellview::bin_fields()
        } else if computer {
            shellview::computer_fields()
        } else if searched {
            shellview::search_fields()
        } else {
            shellview::details_fields()
        };
        if self.sv.fields == want {
            return false;
        }
        if bin {
            self.sv.columns = shellview::bin_columns();
            self.sv.sort = Some((0, true));
        } else if computer {
            self.sv.columns = shellview::computer_columns();
            self.sv.sort = Some((0, true));
        } else if searched {
            self.sv.columns = shellview::search_columns();
            let (col, asc) = self.settings.sort;
            let field = shellview::details_fields().get(col).copied();
            let col = want.iter().position(|f| Some(*f) == field).unwrap_or(0);
            self.sv.sort = Some((col, asc));
        } else {
            self.sv.columns = shellview::details_columns();
            for (col, w) in self.sv.columns.iter_mut().zip(&self.settings.columns) {
                col.width = *w;
            }
            self.sv.sort = Some(self.settings.sort);
        }
        self.sv.fields = want;
        true
    }

    /// Column widths and the sort are saved only for a folder's own columns, never the bin's.
    pub(crate) fn folder_columns(&self) -> bool {
        self.sv.fields == shellview::details_fields()
    }

    /// A newly opened listing's Name starts at its saved width and grows to its longest name
    /// where the window has the room.
    fn fit_name(&mut self, ctx: &Ctx) {
        let default = shellview::details_columns()[0].width;
        let base = match self.folder_columns() {
            true => self.settings.columns.first().copied().unwrap_or(default),
            false => default,
        };
        let frame = self.layout(ctx).view;
        let (cm, px) = (ctx.theme.controls(), ctx.theme.fonts.size_px);
        shellview::fit_name(&mut self.sv, base, frame, &cm, &ctx.text, px);
    }

    /// Fits Details' column `col` to its title and cells, keeping the width for a folder's own
    /// columns.
    pub(crate) fn fit_column(&mut self, col: usize, ctx: &Ctx) -> bool {
        let (p, px, u) = (
            ctx.theme.paint(),
            ctx.theme.fonts.size_px,
            ctx.theme.controls().text,
        );
        if !shellview::fit_column(&mut self.sv, col, &ctx.text, &p, px, u) {
            return false;
        }
        if self.folder_columns() {
            self.settings.columns = self.sv.columns.iter().map(|c| c.width).collect();
            self.settings_dirty = true;
        }
        true
    }

    /// Rescan the drives when `/proc/mounts` changed: the tree, address drop and Recycle Bin
    /// Properties' tabs follow the mounts. Returns whether it rescanned. The scan stats every
    /// mount on a listing thread; one that does not answer (a dead mount) lands through
    /// [`Explorer::take_drives`].
    fn sync_drives(&mut self, ctx: &mut Ctx) -> bool {
        let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
        if mounts == self.mounts_seen {
            self.scanning = None;
            return false;
        }
        if self.scanning.as_ref().is_some_and(|(m, _)| *m == mounts) {
            return false;
        }
        self.bin_drives_changed(ctx);
        match self.read_job(ctx, reprisefs::places::drives) {
            Ok(drives) => {
                self.drives_read(mounts, drives, ctx);
                true
            }
            Err(job) => {
                self.scanning = Some((mounts, job));
                false
            }
        }
    }

    fn drives_read(
        &mut self,
        mounts: String,
        drives: Vec<reprisefs::places::Drive>,
        ctx: &mut Ctx,
    ) {
        self.scanning = None;
        let before = std::mem::take(&mut self.drive_mounts);
        self.drives = drives;
        self.drive_mounts = place::drive_mounts(&self.drives, &self.shares.shares);
        self.mounts_seen = mounts;
        self.mark_volumes();
        if self.drive_mounts != before {
            _ = self.rewalk(&before);
            self.refill_tree_node(&Place::Computer, ctx);
        }
        self.build_address(ctx);
    }

    /// The place's own mount arrived with this scan (a drive opened as it mounted), so the Folders
    /// tree walked to it through Local Disk: that walk folds up where it leaves the new one, and
    /// the tree follows the drive under Computer once Computer lists it. `true` when it did.
    pub(crate) fn rewalk(&mut self, before: &[PathBuf]) -> bool {
        let new = self.place.chain_in(&self.drive_mounts);
        let arrived = (new.get(2).and_then(Place::path))
            .is_some_and(|m| !before.contains(&m) && self.drive_mounts.contains(&m));
        if !arrived {
            return false;
        }
        let old = self.place.chain_in(before);
        if let Some(off) = (0..old.len()).find(|&k| new.get(k) != Some(&old[k]))
            && let Some(path) =
                foldertree::path_of(&self.tree, &self.tree_places, &old[off], before)
            && let Some(node) = self.tree.node_mut(&path)
        {
            node.expanded = false;
            if self
                .tree
                .selected
                .as_ref()
                .is_some_and(|s| s.starts_with(&path))
            {
                self.tree.selected = Some(path);
            }
        }
        self.refollow = true;
        true
    }

    /// Takes a drive scan ([`Explorer::sync_drives`]) that answered late; Computer re-lists on it.
    pub(crate) fn take_drives(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let drives = self.scanning.as_ref()?.1.take()?;
        let (mounts, _) = self.scanning.take()?;
        self.drives_read(mounts, drives, ctx);
        if self.place == Place::Computer {
            return Some(self.refresh(ctx));
        }
        Some(Reply::redraw())
    }

    /// The name `it` shows under: its [`Item::label`], a file's known extension hidden when
    /// `[folders] hide_extensions` asks ([`reprisefs::names::label_for`]).
    pub(crate) fn shown_name(&self, it: &Item) -> String {
        if it.is_dir || it.shortcut.is_some() {
            return it.label().to_string();
        }
        let shown = match self.folders.hide_extensions {
            true => reprisefs::names::Extensions::Hide,
            false => reprisefs::names::Extensions::Show,
        };
        reprisefs::names::label_for(it.label(), &self.globs, shown)
    }

    /// The one selected item, when it opens as a folder.
    pub(crate) fn selected_folder(&self) -> Option<&Item> {
        let i = self.sv.selection.single()?;
        self.items.get(i).filter(|it| it.is_dir)
    }

    /// Opens folder item `it` here or in a new tab. A symlink opens its target, resolved on a
    /// listing thread since the link's mount may be dead; a late answer lands on a wake
    /// ([`Explorer::take_followed`]), dropped if the window has moved on by then.
    pub(crate) fn open_folder(&mut self, it: &Item, to: Follow, ctx: &mut Ctx) -> Reply {
        if let Some((file, inner)) = self.place.archive_entry(&it.path) {
            return self.followed(Place::Archive(file.to_path_buf(), inner), to, ctx);
        }
        if !it.is_link {
            return self.followed(Place::from_path(&it.path), to, ctx);
        }
        let path = it.path.clone();
        match self.read_job(ctx, move || std::fs::canonicalize(&path).unwrap_or(path)) {
            Ok(target) => self.followed(Place::from_path(&target), to, ctx),
            Err(job) => {
                self.following = Some((self.place.clone(), to, job));
                Reply::none()
            }
        }
    }

    pub(crate) fn take_followed(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let target = self.following.as_ref()?.2.take()?;
        let (from, to, _) = self.following.take()?;
        (from == self.place).then(|| self.followed(Place::from_path(&target), to, ctx))
    }

    fn followed(&mut self, place: Place, to: Follow, ctx: &mut Ctx) -> Reply {
        match (to, place.path()) {
            (Follow::NewWindow, Some(dir)) => spawn_in(
                &[std::ffi::OsStr::new("reprise-explorer"), dir.as_os_str()],
                "reprise-explorer",
            ),
            (Follow::Here | Follow::NewWindow, _) => self.go(place, ctx),
            (Follow::NewTab, _) => self.open_in_new_tab(place),
        }
    }

    /// In the Recycle Bin an item opens its Properties (its menu's default
    /// row): a deleted file is not opened until restored.
    pub fn open_selected(&mut self, ctx: &mut Ctx) -> Reply {
        let how = Confirm::on_open(repriseconf::current().confirm_scripts);
        self.open_selected_with(repriseconf::current().script_open, how, ctx)
    }

    /// [`Explorer::open_selected`] from a Run row: a script runs as `open` says, unasked.
    pub fn open_selected_as(&mut self, open: ScriptOpen, ctx: &mut Ctx) -> Reply {
        self.open_selected_with(open, Confirm::Skip, ctx)
    }

    fn open_selected_with(&mut self, open: ScriptOpen, how: Confirm, ctx: &mut Ctx) -> Reply {
        let Some(i) = self.sv.selection.single() else {
            return Reply::none();
        };
        if self.place == Place::RecycleBin {
            return self.properties(ctx);
        }
        let Some(it) = self.items.get(i).cloned() else {
            return Reply::none();
        };
        if let Some(uri) = self.selected_network() {
            return self.open_network(&uri, ctx);
        }
        if let Some(v) = self.selected_unmounted() {
            return self.mount_and_open(v, ctx);
        }
        if let Some(d) = self.selected_unmounted_device() {
            return self.mount_device_and_open(d, ctx);
        }
        if self.selected_disk_image().is_some() {
            return self.ask_mount_image(ctx);
        }
        if it.is_dir {
            let to = match self.folders.new_window {
                true => Follow::NewWindow,
                false => Follow::Here,
            };
            return self.open_folder(&it, to, ctx);
        }
        if let Some(shortcut) = it.shortcut.as_deref() {
            return self.opened(&it.path, Opened::Open(self.open_for(shortcut)), ctx);
        }
        if let Some((file, path)) = self.place.archive_entry(&it.path) {
            let entry = reprisefs::archive::Entry {
                path,
                kind: reprisefs::archive::Kind::File,
                size: it.size,
                modified: it.modified,
                mode: 0o644,
            };
            let (file, open) = (file.to_path_buf(), self.opening(false, open, how));
            let key = self.archives.key(&file);
            let globs = self.globs.clone();
            let copy = move || {
                let dir = match reprisefs::archive::scratch() {
                    Ok(dir) => dir,
                    Err(e) => return Opened::NotCopied(e.to_string()),
                };
                let copied = reprisefs::archive::copy_out((&file, key.as_ref()), &entry, &dir);
                drop(key);
                let copy = match copied {
                    Ok(copy) => copy,
                    Err(why) => {
                        let _ = std::fs::remove_dir(&dir);
                        return Opened::NotCopied(why);
                    }
                };
                if let Err(e) = reprisefs::archivecopy::set_origin(&copy, &file, &entry.path) {
                    log::debug!(
                        "{} keeps no note of {}: {e}",
                        copy.display(),
                        file.display()
                    );
                }
                let name = entry.path.file_name().unwrap_or_default().to_string_lossy();
                if reprisefs::archive::runs_as_a_program(
                    &copy,
                    globs.mime_of_name(&name).as_deref(),
                ) {
                    let _ = std::fs::remove_dir_all(&dir);
                    return Opened::Runs;
                }
                Opened::Copied(copy.clone(), Box::new(open(copy)))
            };
            return self.open_off_loop(it.path.clone(), copy, ctx);
        }
        self.open_file(&it.path, it.exec, open, how, ctx)
    }

    /// Starts the configured terminal in `dir`, in its program's app scope, once its token is in
    /// ([`Explorer::launch`]).
    pub(crate) fn open_terminal(&mut self, dir: &Path) -> Reply {
        let terminal = repriseconf::current().terminal.clone();
        let app = reprisefs::apps::terminal_app(&terminal, &self.app_index);
        let argv = reprisefs::apps::terminal_argv(&terminal);
        let argv = reprisefs::scope::in_scope(&argv, app.as_ref());
        self.launches
            .push_back(Launch::Terminal(argv, dir.to_path_buf()));
        Reply {
            handoffs: 1,
            ..Reply::none()
        }
    }

    /// Starts the oldest launch waiting with `handoff`, the token its reply asked for. The spawn
    /// runs on a listing thread: it waits for the child's chdir, which never returns on a dead
    /// mount, and a run's for its app scope on the session bus.
    pub(crate) fn launch(&mut self, handoff: Handoff, ctx: &mut Ctx) -> Reply {
        match self.launches.pop_front() {
            None => Reply::none(),
            Some(Launch::Terminal(argv, dir)) => {
                let job = self.spawn_read(ctx, move || {
                    if let Err(e) = repriseapp::spawn_handed(&argv, Some(&dir), None, &handoff) {
                        log::warn!("explorer: terminal {argv:?} in {}: {e}", dir.display());
                    }
                });
                self.starting.push(job);
                Reply::none()
            }
            Some(Launch::Run(path, argv, scope)) => {
                let dir = path.parent().map(Path::to_path_buf);
                let run = move || {
                    let ran =
                        repriseapp::spawn_handed(&argv, dir.as_deref(), scope.as_ref(), &handoff);
                    Opened::Ran(ran)
                };
                self.open_off_loop(path, run, ctx)
            }
        }
    }

    /// Runs the program file at `path`, or opens another file in its type's program, unless `how`
    /// asks first ([`Explorer::ask_run`]). Reads the file off the loop ([`Explorer::take_opened`]).
    pub(crate) fn open_file(
        &mut self,
        path: &Path,
        exec: bool,
        open: ScriptOpen,
        how: Confirm,
        ctx: &mut Ctx,
    ) -> Reply {
        let (path, open) = (path.to_path_buf(), self.opening(exec, open, how));
        let at = path.clone();
        self.open_off_loop(at, move || open(path), ctx)
    }

    /// What opening the file at a path comes to, for a listing thread: [`Explorer::open_file`]'s
    /// reading of it.
    fn opening(
        &self,
        exec: bool,
        open: ScriptOpen,
        how: Confirm,
    ) -> impl FnOnce(PathBuf) -> Opened + Send + 'static {
        let (globs, apps) = (self.globs.clone(), self.app_index.clone());
        let (terminal, end) = (
            repriseconf::current().terminal.clone(),
            repriseconf::current().script_terminal,
        );
        move |path: PathBuf| {
            let program = exec
                .then(|| std::fs::metadata(&path).ok())
                .flatten()
                .and_then(|m| reprisefs::program::program(&path, &m));
            let Some(program) = program else {
                let mime = reprisefs::apps::mime_of_in(&globs, &path);
                if reprisefs::program::opens_as_a_program(&path, mime.as_deref())
                    && reprisefs::program::asks_first(&path, how)
                {
                    return Opened::Ask(open, reprisefs::program::Asked::Typed);
                }
                if mime
                    .as_deref()
                    .is_some_and(|m| reprisefs::archive::opens_as_folder(m, &apps))
                {
                    return Opened::Browse;
                }
                let open = reprisefs::props::open_in(&path, &globs, &apps, &terminal);
                let Some(open) = open.transpose() else {
                    return Opened::Choose;
                };
                if let Ok(open) = &open
                    && repriseconf::current().recent_files
                {
                    let app = (open.app.as_ref()).and_then(|id| apps.iter().find(|a| a.id == *id));
                    if let Err(e) = reprisefs::recent::record(&[(&path, app, mime.as_deref())]) {
                        log::warn!("recent files: could not record {}: {e}", path.display());
                    }
                }
                return Opened::Open(open);
            };
            if reprisefs::program::asks_first(&path, how) {
                return Opened::Ask(open, reprisefs::program::Asked::of(program));
            }
            let scope =
                (path.file_name()).and_then(|n| reprisefs::apps::AppId::new(&n.to_string_lossy()));
            match reprisefs::program::run_argv(&path, open, &terminal, end) {
                Ok(argv) => Opened::Run(argv, scope),
                Err(e) => Opened::Ran(Err(e)),
            }
        }
    }

    /// Runs `read` for the opening of `at` on a listing thread, answered now or, late, by
    /// [`Explorer::take_opened`].
    fn open_off_loop(
        &mut self,
        at: PathBuf,
        read: impl FnOnce() -> Opened + Send + 'static,
        ctx: &mut Ctx,
    ) -> Reply {
        match self.read_job(ctx, read) {
            Ok(opened) => self.opened(&at, opened, ctx),
            Err(job) => {
                self.opening.push((at, job));
                Reply::none()
            }
        }
    }

    /// Takes the openings ([`Explorer::open_file`]) that answered late, their spawns joined as
    /// [`Reply::and`] joins them.
    pub(crate) fn take_opened(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let mut done = Vec::new();
        self.opening.retain(|(path, job)| match job.take() {
            Some(opened) => {
                done.push((path.clone(), opened));
                false
            }
            None => true,
        });
        if done.is_empty() {
            return None;
        }
        let mut reply = Reply::redraw();
        for (path, opened) in done {
            let r = self.opened(&path, opened, ctx);
            reply.redraw_all |= r.redraw_all;
            reply = reply.and(Reply {
                spawn: r.spawn,
                spawn_app: r.spawn_app,
                spawn_dir: r.spawn_dir,
                handoffs: r.handoffs,
                ..Reply::none()
            });
        }
        Some(reply)
    }

    fn opened(&mut self, path: &Path, opened: Opened, ctx: &mut Ctx) -> Reply {
        match opened {
            Opened::Open(Ok(open)) => Reply {
                redraw: true,
                spawn: vec![open.argv],
                spawn_app: open.app.map(|a| a.app_id()),
                spawn_dir: open.dir,
                ..Default::default()
            },
            Opened::Open(Err(refused)) => self.exec_refused(&refused, ctx),
            Opened::Browse => self.go(Place::Archive(path.to_path_buf(), PathBuf::new()), ctx),
            Opened::Choose => self.choose_program_for(path, None, ctx),
            Opened::Copied(copy, opened) => self.opened(&copy, *opened, ctx),
            Opened::Runs => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                let text = tr!("archive-runs", name = &*name);
                self.message_box(&tr!("archive-copy-failed-title"), text, ctx)
            }
            Opened::NotCopied(why) => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                let text = tr!("archive-copy-failed", name = &*name, why = why);
                self.message_box(&tr!("archive-copy-failed-title"), text, ctx)
            }
            Opened::Run(argv, scope) => {
                self.launches
                    .push_back(Launch::Run(path.to_path_buf(), argv, scope));
                Reply {
                    handoffs: 1,
                    ..Reply::redraw()
                }
            }
            Opened::Ran(Ok(())) => Reply::redraw(),
            Opened::Ask(open, asked) => self.ask_run(path, open, asked, ctx),
            Opened::Ran(Err(e)) => {
                log::warn!("run {}: {e}", path.display());
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                let text = reprisefs::program::run_failed_text(&name, &e);
                self.message_box(&name, text, ctx)
            }
        }
    }

    /// Re-order by the current sort column. Every mode shows the same order, so
    /// this runs on every listing and header click. A grouped listing sorts
    /// within each group. The selection is kept by path, and a rename box moves with its item.
    pub fn resort(&mut self) {
        let Some((col, asc)) = self.sv.sort else {
            self.index_items();
            return;
        };
        let chosen = self.selection_paths();
        let firsts: Vec<usize> = self.sv.groups.iter().map(|g| g.first).collect();
        let field = self.sv.fields.get(col).copied().unwrap_or(Field::Name);
        let order = sort_order_grouped(&self.items, &firsts, field, asc, &self.spaces);
        let mut items: Vec<Option<Item>> = std::mem::take(&mut self.items)
            .into_iter()
            .map(Some)
            .collect();
        let mut entries: Vec<Option<Entry>> = std::mem::take(&mut self.sv.entries)
            .into_iter()
            .map(Some)
            .collect();
        self.items = order
            .iter()
            .map(|&i| items[i].take().expect("each index once"))
            .collect();
        self.sv.hot = None;
        self.sv.entries = order
            .iter()
            .map(|&i| entries[i].take().expect("each index once"))
            .collect();
        if let Some((i, _)) = self.sv.edit.as_mut()
            && let Some(j) = order.iter().position(|&o| o == *i)
        {
            *i = j;
        }
        self.index_items();
        self.select_paths(&chosen);
    }

    fn index_items(&mut self) {
        self.item_index = self
            .items
            .iter()
            .enumerate()
            .map(|(i, it)| (it.path.clone(), i))
            .collect();
    }

    pub(crate) fn index_of(&self, path: &Path) -> Option<usize> {
        self.item_index.get(path).copied()
    }

    /// The entries `--select` names, the first focused and scrolled into view. One the listing
    /// doesn't show (a hidden file) stays unselected.
    pub(crate) fn select_shown(&mut self, paths: &[PathBuf], ctx: &mut Ctx) {
        let shown: Vec<PathBuf> = paths
            .iter()
            .filter(|p| self.index_of(p).is_some())
            .cloned()
            .collect();
        if shown.is_empty() {
            return;
        }
        self.select_paths(&shown);
        self.sv.selection.focus = self.index_of(&shown[0]);
        self.sv.selection.anchor = self.sv.selection.focus;
        let lay = self.layout(ctx);
        let sl = self.view_layout(lay.view, ctx);
        shellview::follow(&mut self.sv, &sl);
        self.refresh_chrome(ctx);
    }

    /// Select exactly `paths`, focusing the first; what a re-listing or sort
    /// keeps selected. Keyed by path, not shown name: two non-UTF-8 names can
    /// show as the same text.
    pub(crate) fn select_paths(&mut self, paths: &[PathBuf]) {
        self.sv.selection.clear();
        for p in paths {
            if let Some(i) = self.index_of(p) {
                self.sv.selection.set.insert(i);
            }
        }
        self.sv.selection.focus = self.sv.selection.set.iter().next().copied();
        self.sv.selection.anchor = self.sv.selection.focus;
    }
}

/// Explorer needs the icon cache to build itself, which arrives with the first event.
struct Lazy {
    start: Start,
    inner: Option<Explorer>,
    /// How the text grows as the program starts, for the window's opening size.
    units: TextUnits,
}

impl Explorer {
    /// An event on window `win`. After every event a paste whose turn came asks its clash, and a
    /// progress window held still on a box takes a tick once its job moved.
    pub(crate) fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        let mut reply = route(self, win, ev, ctx);
        // The task pane draws its focus rectangle only while it holds the keyboard.
        if self.focus != Focus::Tasks {
            self.taskpane.focus = None;
        }
        // The pointer or the keyboard came to or left the navigation pane, or its fade ticked.
        if win == MAIN {
            let (faded, fading) = self.nav_fade();
            reply.redraw |= faded;
            reply.tick |= fading;
        }
        let mut reply = self.raise_waiting(reply, ctx);
        reply.tick_wins.extend(self.held_progress_moved());
        reply
    }
}

fn route(ex: &mut Explorer, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
    ex.follow_forms(ctx);
    ex.follow_look(ctx);
    match ev {
        Event::Key(_) => ex.keyed = true,
        Event::Pointer(PointerInput::Press { .. }) => ex.keyed = false,
        _ => {}
    }
    // An Alt held when a child window came up is released over there; that
    // release must not come back as a menu-bar activation.
    if win != MAIN {
        ex.alt_arm.interrupt();
    }
    if is_dialog(win) {
        // An event for a box that closed, such as a held Enter's repeat, is not the next box's.
        if win != ex.dialog_win() {
            return Reply::none();
        }
        return ex.dialog_event(ev, ctx);
    }
    if win == PROGRAM_BROWSE {
        return ex.program_browse_event(ev, ctx);
    }
    if win == props_picture::PICTURE_BROWSE {
        return ex.picture_browse_event(ev, ctx);
    }
    if win == browse::ERROR {
        return ex.browse_error_event(ev);
    }
    if win == PROPS {
        return ex.props_event(ev, ctx);
    }
    if win == bin_props::BIN_PROPS {
        return ex.bin_props_event(ev, ctx);
    }
    if win == folder_opts::FOLDER_OPTS {
        return ex.folder_opts_event(ev, ctx);
    }
    if win == preview::PREVIEW {
        return ex.preview_event(ev, ctx);
    }
    if [copydlg::COPY, copydlg::REMOVE, copydlg::EXTRACT].contains(&win) {
        return ex.progress_event(win, ev, ctx);
    }
    let reply = match ev {
        Event::Key(k) => ex.key(k, ctx),
        Event::KeyUp(k) => {
            // A release whose press never arrived is a key the compositor grabbed
            // (Alt+space opens its window menu): this Alt means nothing here.
            if k.sym != keysym::ALT_L && k.sym != keysym::ALT_R {
                ex.alt_arm.interrupt();
                return Reply::none();
            }
            // Only a bare Alt press and release with the window's own keyboard activates
            // the bar; a popup or modal child owns the keys while up.
            if ex.alt_arm.release_alt()
                && ex.popup.is_none()
                && ex.dialog().is_none()
                && ex.props.is_none()
                && ex.bin_props.is_none()
            {
                let lay = ex.bar_layout(ctx);
                let m = ctx.theme.clone();
                ex.bar
                    .key(Key::Activate, &lay, &ctx.text, &m, ctx.theme.fonts.size_px);
                return Reply::redraw();
            }
            Reply::none()
        }
        Event::Pointer(p) => ex.pointer(p, ctx),
        Event::PastedFiles(clip) => ex.pasted(clip, ctx),
        Event::PastedImage(bytes) => ex.pasted_image(bytes, ctx),
        Event::Pasted(text) => ex.paste_text(&text),
        Event::Wake => ex.wake(ctx),
        Event::Handoff(handoff) => ex.launch(handoff, ctx),
        Event::Blink => {
            ex.flush_settings();
            // The Address and Search fields' carets are the only things in the window that blink.
            Reply {
                redraw: ex.keyboard && matches!(ex.focus, Focus::Address | Focus::Search),
                ..Reply::none()
            }
        }
        Event::Tick => {
            let field = ex.address_tick(ctx);
            let tip = ex.tip_tick(std::time::Instant::now(), ctx);
            Reply {
                redraw: field.redraw || tip.redraw,
                tick: field.tick || tip.tick,
                ..tip
            }
        }
        Event::Focus(false) => {
            ex.keyboard = false;
            ex.alt_arm.interrupt();
            let lit = ex.bar.is_active();
            ex.bar.close();
            let closed = ex.close_drop_menu();
            let tipped = ex.hide_tip();
            ex.flush_settings();
            Reply {
                redraw: lit
                    || closed
                    || tipped
                    || matches!(ex.focus, Focus::Address | Focus::Search),
                ..Reply::none()
            }
        }
        Event::Focus(true) => {
            ex.keyboard = true;
            ex.keyboard_entered(ctx).and(Reply::redraw())
        }
        // A maximized window's size is the screen's: a hand size still pending is saved.
        Event::Resized if ctx.maximized => Reply::redraw(),
        // Only a size the user gives is kept: the opening configure leaves the file alone.
        Event::Resized => {
            ex.window_px = (!ctx.at_pick).then_some(ctx.size);
            ex.settings_dirty |= ex.window_px.is_some();
            Reply::redraw()
        }
        Event::CloseRequest => ex.close(ctx),
        Event::DragEnter {
            x, y, offered, own, ..
        } => ex.drag_enter(x, y, offered, own, ctx),
        Event::DragFiles { paths } => ex.drag_files(paths, ctx),
        Event::DragMotion { x, y, .. } => ex.drag_over(x, y, ctx),
        Event::DragLeave => ex.drag_leave(),
        Event::DragDrop {
            x,
            y,
            data,
            action,
            source,
        } => ex.drag_drop((x, y), data, action, source, ctx),
        Event::DropCancelled => ex.drop_cancelled(),
        Event::DragEnd { .. } => ex.drag_ended(),
        _ => Reply::redraw(),
    };
    ex.settle_drop(reply)
}

impl App for Lazy {
    fn title(&self) -> String {
        match &self.inner {
            Some(ex) => {
                (ex.search.caption(&ex.shares.shares)).unwrap_or_else(|| ex.title_of(&ex.place))
            }
            None => self.start.place.title(),
        }
    }

    fn app_id(&self) -> String {
        "reprise.explorer".to_string()
    }

    fn initial_size(&self) -> (i32, i32) {
        match &self.inner {
            Some(ex) => ex.settings.window_at(ex.forms.text),
            None => Settings::load().window_at(self.units),
        }
    }

    fn children(&self) -> Vec<Child> {
        match &self.inner {
            Some(ex) => ex.children(),
            None => Vec::new(),
        }
    }

    fn start(&mut self, waker: repriseapp::Waker) -> repriseapp::Start {
        // A start with no folder window has no window to bring the first event.
        if self.start.alone.is_some() {
            waker.wake();
        }
        repriseapp::Start::Continue
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        let start = &self.start;
        let mut opened = Reply::none();
        let ex = match &mut self.inner {
            Some(ex) => ex,
            None => {
                let ex = self.inner.insert(Explorer::open(
                    start.clone(),
                    |d| d.is_dir(),
                    Explorer::new,
                    ctx,
                ));
                opened = match &start.alone {
                    Some(Alone::Properties(paths)) => ex.open_paths_properties(paths, ctx),
                    Some(Alone::OpenWith(path)) => ex.choose_program_for(path, None, ctx),
                    Some(Alone::NewShortcut(dir)) => ex.new_shortcut_in(dir.clone(), ctx),
                    Some(Alone::BinProperties) => ex.open_bin_props(ctx),
                    Some(Alone::Put {
                        op,
                        target,
                        sources,
                    }) => ex.put_paths(*op, sources, target, ctx),
                    Some(Alone::AddToZip { zip, sources }) => {
                        ex.put_in_zip(sources, (zip, Path::new("")), ctx)
                    }
                    Some(Alone::Delete { paths, erase }) => {
                        ex.confirm_delete_paths(paths.clone(), *erase, ctx)
                    }
                    Some(Alone::EmptyBin) => ex.confirm_empty_bins(ctx),
                    Some(Alone::Zip(paths)) => ex.zip_paths(paths.clone(), ctx),
                    Some(Alone::Undo(entry)) => ex.undo_with(*entry, false, ctx),
                    None => Reply::none(),
                };
                ex
            }
        };
        let reply = opened.and(ex.event(win, ev, ctx));
        ex.hold_logout();
        if start.alone.is_some() && !ex.closing && ex.sheet_closed() {
            return reply.and(ex.close(ctx));
        }
        if ex.closing && !ex.owes_a_box(ctx) && !ex.busy() {
            return reply.and(Reply {
                quit: true,
                ..Default::default()
            });
        }
        reply
    }

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        let Some(ex) = &mut self.inner else {
            c.fill_rect(c.bounds(), ctx.theme.colors.button_face);
            return;
        };
        ex.follow_forms(ctx);
        ex.follow_look(ctx);
        if is_dialog(win) {
            ex.draw_dialog(c, ctx);
            return;
        }
        if win == PROGRAM_BROWSE {
            ex.draw_program_browse(c);
            return;
        }
        if win == props_picture::PICTURE_BROWSE {
            ex.draw_picture_browse(c);
            return;
        }
        if win == browse::ERROR {
            ex.draw_browse_error(c, ctx);
            return;
        }
        if win == PROPS {
            ex.draw_props(c, ctx);
            return;
        }
        if win == bin_props::BIN_PROPS {
            ex.draw_bin_props(c, ctx);
            return;
        }
        if win == folder_opts::FOLDER_OPTS {
            ex.draw_folder_opts(c, ctx);
            return;
        }
        if win == preview::PREVIEW {
            ex.draw_preview(c, ctx);
            return;
        }
        if [copydlg::COPY, copydlg::REMOVE, copydlg::EXTRACT].contains(&win) {
            ex.draw_progress(win, c);
            return;
        }
        ex.main_size = ctx.size;
        ex.status.grip = !ctx.maximized;
        // A look that boxes its items greys the unfocused pane's selection; Classic keeps the highlight.
        let boxed = item::boxed(&ctx.theme.paint());
        ex.tree.inactive = boxed && ex.focus != Focus::Tree;
        ex.sv.inactive = boxed && ex.focus != Focus::View;
        window::draw(ex, c, ctx);
    }
}

impl repriseapp::Extended for Lazy {
    /// The folder window's tabs, or its place when it has one tab; a file job or sheet started
    /// alone has none.
    fn reopen(&self, win: WinId) -> Option<String> {
        if win != MAIN || self.start.alone.is_some() {
            return None;
        }
        match (&self.inner, &self.start.tabs) {
            (Some(ex), _) => ex.restored().arg(),
            (None, Some(tabs)) => tabs.arg(),
            (None, None) => reopen_arg(&self.start.place),
        }
    }

    fn hidden(&self) -> bool {
        self.start.alone.is_some() || self.inner.as_ref().is_some_and(|ex| ex.closing)
    }

    fn parent(&self, win: WinId) -> Option<WinId> {
        Some(self.inner.as_ref().map_or(MAIN, |ex| ex.parent_of(win)))
    }

    fn caption(&self, win: WinId) -> repriseapp::Caption {
        self.inner
            .as_ref()
            .map_or(repriseapp::Caption::Window, |ex| ex.caption(win))
    }

    /// The shell layout's nav row names the place, so its caption shows neither the icon nor
    /// the title; asked before the first event too, as the window is made.
    fn bare_caption(&self, win: WinId) -> bool {
        win == MAIN
            && match &self.inner {
                Some(ex) => ex.shell_on,
                None => window::is_shell(&repriseapp::start_forms()),
            }
    }

    fn min_size(&self, win: WinId) -> Option<(i32, i32)> {
        self.inner.as_ref().and_then(|ex| ex.min_size(win))
    }

    /// The main window shows the icon of the place it shows, as the address bar does.
    fn window_icon(&self, win: WinId) -> Option<&'static str> {
        if win == MAIN {
            let place = self
                .inner
                .as_ref()
                .map_or(&self.start.place, |ex| &ex.place);
            return Some(place.icon_name());
        }
        let b = self.inner.as_ref()?.bin_props.as_ref()?;
        (win == bin_props::BIN_PROPS).then(|| b.icon())
    }

    fn popup(&mut self, win: WinId, ctx: &Ctx) -> Option<Box<dyn Hosted + '_>> {
        if win != MAIN {
            return repriseapp::reader_popup(self, win, ctx);
        }
        let ex = self.inner.as_mut()?;
        if ex.bar.open.is_some() {
            return Some(Box::new(&mut ex.bar));
        }
        if ex.popup.is_some() {
            return Some(Box::new(&mut ex.popup));
        }
        if !ex.address.complete.is_open() {
            ex.address.combo.open?;
        }
        let (px, cm) = (ctx.theme.fonts.size_px, ctx.theme.controls());
        let field = ex.address_layout(ctx)?.field;
        if ex.address.complete.is_open() {
            return Some(Box::new(pathcomplete::Popped {
                pc: &mut ex.address.complete,
                field,
                cm,
                px,
            }));
        }
        Some(Box::new(combo::Popped {
            combo: &mut ex.address.combo,
            field,
            cm,
            px,
        }))
    }

    fn drop_form(&mut self, win: WinId) -> Option<DropForm<'_>> {
        let form = match win {
            PROPS => self.inner.as_mut()?.props.as_mut()?.sheet.open_mut(),
            bin_props::BIN_PROPS => self
                .inner
                .as_mut()?
                .bin_props
                .as_mut()?
                .sheet
                .sheet
                .open_mut(),
            _ => return None,
        };
        Some(DropForm { form, px: None })
    }

    fn reader(&mut self, win: WinId, act: repriseapp::Reader, ctx: &Ctx) -> bool {
        match (win, &mut self.inner) {
            (MAIN, Some(ex)) => window::reader(ex, act, ctx),
            (w, Some(ex)) if is_dialog(w) => ex.chooser_reader(act),
            _ => repriseapp::reader_forms(self, win, act, ctx),
        }
    }

    fn cancel_pointer(&mut self, win: WinId) {
        match (win, &mut self.inner) {
            (MAIN, Some(ex)) => ex.cancel_pointer(),
            _ => repriseapp::reader_cancel_pointer(self, win),
        }
    }

    fn drop_file_dialog(&mut self, win: WinId) -> Option<FileDrop<'_>> {
        let ex = self.inner.as_mut()?;
        let b = match win {
            PROGRAM_BROWSE => crate::newshortcut::picker_of_mut(ex.dialogs.up_mut()?)?,
            props_picture::PICTURE_BROWSE => ex.picture_browse_mut()?,
            _ => return None,
        };
        Some(FileDrop {
            dlg: &mut b.dlg,
            host: &mut b.host,
        })
    }

    fn follow_scale(&mut self, win: WinId, scale: i32, ctx: &mut Ctx) {
        if win == MAIN
            && let Some(ex) = &mut self.inner
        {
            ex.follow_scale(scale, ctx);
        }
    }

    fn drag_image(&mut self, win: WinId, scale: i32, ctx: &mut Ctx) -> Option<Canvas> {
        let ex = self.inner.as_mut().filter(|_| win == MAIN)?;
        ex.drag_image(ctx, scale).map(|(image, _)| image)
    }

    fn access(&mut self, win: WinId, tree: &mut reprisekit::access::Tree, ctx: &Ctx) {
        let Some(ex) = &mut self.inner else {
            return;
        };
        ex.follow_forms(ctx);
        match win {
            w if is_dialog(w) => ex.access_dialog(tree, ctx),
            copydlg::COPY | copydlg::REMOVE | copydlg::EXTRACT => ex.access_progress(win, tree),
            PROGRAM_BROWSE => ex.access_program_browse(tree),
            props_picture::PICTURE_BROWSE => ex.access_picture_browse(tree),
            browse::ERROR => ex.access_browse_error(tree),
            preview::PREVIEW => ex.access_preview(tree, ctx),
            PROPS | bin_props::BIN_PROPS | folder_opts::FOLDER_OPTS => {
                let sheet = match win {
                    PROPS => ex.props.as_ref().map(|p| &p.sheet),
                    bin_props::BIN_PROPS => ex.bin_props.as_ref().map(|b| &b.sheet.sheet),
                    _ => ex.folder_opts.as_ref().map(|f| &f.sheet.sheet),
                };
                if let Some(sheet) = sheet {
                    let px = ctx.forms().fonts.size_px;
                    let (_, focus) = sheet.access(tree, reprisekit::access::ROOT, &ctx.text, px);
                    if let Some(f) = focus {
                        tree.set_focus(f);
                    }
                }
            }
            _ => window::access(ex, tree, ctx),
        }
    }
}

/// What the command line asks for, read without touching the file system: the place, the
/// folder still to be checked to exist, and the entry to select once it is listed.
#[derive(Clone, Debug, PartialEq)]
struct Start {
    place: Place,
    dir: Option<PathBuf>,
    select: Vec<PathBuf>,
    /// A server address to connect to once the window is up.
    connect: Option<String>,
    /// The box that is the only window; the process ends with it.
    alone: Option<Alone>,
    /// `--search`: the keyboard starts in the Search field.
    search: bool,
    /// `--tabs=`: the window's tabs at session restore, `place` the shown one's.
    tabs: Option<tabs::Restored>,
}

/// A start that shows one box, or runs one file job, without a folder window.
#[derive(Clone, Debug, PartialEq)]
enum Alone {
    /// `--properties PATH...`: the sheet of one path, or the combined sheet of several.
    Properties(Vec<PathBuf>),
    /// `--open-with PATH`: the Open With chooser.
    OpenWith(PathBuf),
    /// `--new-shortcut DIR`: New > Shortcut's wizard, writing into folder DIR.
    NewShortcut(PathBuf),
    /// `--bin-properties`: Recycle Bin Properties.
    BinProperties,
    /// `--put copy|move|link TARGET -- SOURCES...`: the SOURCES pasted into folder TARGET.
    Put {
        op: DropOp,
        target: PathBuf,
        sources: Vec<PathBuf>,
    },
    /// `--add-to-zip ZIP -- SOURCES...`: copies of the SOURCES dropped onto the zip ZIP.
    AddToZip { zip: PathBuf, sources: Vec<PathBuf> },
    /// `--delete [--erase] PATHS...`: Delete on the PATHS, Shift+Delete with `--erase`.
    Delete { paths: Vec<PathBuf>, erase: bool },
    /// `--empty-bin`: Empty Recycle Bin.
    EmptyBin,
    /// `--zip PATHS...`: Send To > Compressed (zipped) Folder on the PATHS.
    Zip(Vec<PathBuf>),
    /// `--undo ENTRY`: Undo on the session's history, of that entry while it is the top one.
    Undo(reprisefs::undo::EntryId),
}

/// The rest of the command line as paths: None unless there is one and every one is absolute.
fn absolute_paths(args: impl Iterator<Item = std::ffi::OsString>) -> Option<Vec<PathBuf>> {
    let paths: Vec<PathBuf> = args.map(PathBuf::from).collect();
    (!paths.is_empty() && paths.iter().all(|p| p.is_absolute())).then_some(paths)
}

/// `--put`'s arguments: the op, the absolute TARGET, `--` and the absolute SOURCES.
fn put_args(mut args: impl Iterator<Item = std::ffi::OsString>) -> Option<Alone> {
    let op = DropOp::from_arg(args.next()?.to_str()?)?;
    let target = PathBuf::from(args.next()?);
    if !target.is_absolute() || args.next()? != "--" {
        return None;
    }
    Some(Alone::Put {
        op,
        target,
        sources: absolute_paths(args)?,
    })
}

/// `--add-to-zip`'s arguments: the absolute ZIP, `--` and the absolute SOURCES.
fn add_to_zip_args(mut args: impl Iterator<Item = std::ffi::OsString>) -> Option<Alone> {
    let zip = PathBuf::from(args.next()?);
    if !zip.is_absolute() || args.next()? != "--" {
        return None;
    }
    Some(Alone::AddToZip {
        zip,
        sources: absolute_paths(args)?,
    })
}

/// `reprise-explorer [--search] [PLACE]`, `--select PATH...`, `--properties PATH...`,
/// `--open-with PATH`, `--new-shortcut DIR`, `--bin-properties` or one of [`Alone`]'s file jobs. PLACE is a
/// folder's path, a place name or URI ([`Place::read_address`]), or a server address, which opens
/// Computer and connects to it; a missing or unknown one is Home. `--search` puts the keyboard in
/// the Search field. `--select` opens the folder the first absolute PATH sits in with it and the
/// others there selected, the first scrolled into view (Find Target's window). `--properties`
/// shows the Properties sheet of the absolute PATHs alone, combined for two or more;
/// `--open-with` shows the Open With chooser for PATH alone, `--new-shortcut` New > Shortcut's
/// wizard writing into the absolute DIR, and `--bin-properties` Recycle Bin Properties. A file job runs with its questions, progress and error boxes, and the process
/// ends with it.
fn parse_args(args: impl IntoIterator<Item = std::ffi::OsString>) -> Start {
    let mut args = args.into_iter();
    let first = args.next();
    if let Some(tabs) = first
        .as_ref()
        .and_then(|a| tabs::Restored::parse(a.to_str()?))
    {
        let place = tabs.list[tabs.open].0.clone();
        return Start {
            dir: place.path(),
            place,
            select: Vec::new(),
            connect: None,
            alone: None,
            search: false,
            tabs: Some(tabs),
        };
    }
    if first.as_deref() == Some("--archive".as_ref()) {
        let file = args.next().map(PathBuf::from).filter(|p| p.is_absolute());
        return Start {
            place: file.map_or(Place::Home, |f| Place::Archive(f, PathBuf::new())),
            ..parse_args(std::iter::empty())
        };
    }
    if first.as_deref() == Some("--search".as_ref()) {
        return Start {
            search: true,
            ..parse_args(args)
        };
    }
    if let Some(addr) = first
        .as_ref()
        .and_then(|a| a.to_str())
        .and_then(Address::parse)
    {
        return Start {
            place: Place::Computer,
            dir: None,
            select: Vec::new(),
            connect: Some(addr.uri()),
            alone: None,
            search: false,
            tabs: None,
        };
    }
    let alone = match first.as_ref().and_then(|a| a.to_str()) {
        Some("--properties") => {
            let paths: Vec<PathBuf> = args.by_ref().map(PathBuf::from).collect();
            (paths.iter().all(|p| p.is_absolute()) && !paths.is_empty())
                .then_some(Alone::Properties(paths))
        }
        Some("--open-with") => args
            .next()
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .map(Alone::OpenWith),
        Some("--new-shortcut") => args
            .next()
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .map(Alone::NewShortcut),
        Some("--bin-properties") => Some(Alone::BinProperties),
        Some("--put") => put_args(args.by_ref()),
        Some("--add-to-zip") => add_to_zip_args(args.by_ref()),
        Some("--delete") => {
            let mut rest = args.by_ref().peekable();
            let erase = rest.next_if(|a| a == "--erase").is_some();
            absolute_paths(rest).map(|paths| Alone::Delete { paths, erase })
        }
        Some("--empty-bin") => Some(Alone::EmptyBin),
        Some("--zip") => absolute_paths(args.by_ref()).map(Alone::Zip),
        Some("--undo") => (args.next())
            .and_then(|a| a.to_str()?.parse().ok())
            .map(Alone::Undo),
        _ => None,
    };
    if let Some(alone) = alone {
        let place = match &alone {
            Alone::Properties(paths) => paths[0].parent().map_or(Place::Home, Place::from_path),
            Alone::OpenWith(path) => path.parent().map_or(Place::Home, Place::from_path),
            Alone::NewShortcut(dir) => Place::from_path(dir),
            Alone::BinProperties | Alone::EmptyBin => Place::RecycleBin,
            Alone::Undo(_) => Place::Home,
            Alone::Put { target, .. } => Place::from_path(target),
            Alone::AddToZip { zip, .. } => zip.parent().map_or(Place::Home, Place::from_path),
            Alone::Delete { paths, .. } | Alone::Zip(paths) => {
                paths[0].parent().map_or(Place::Home, Place::from_path)
            }
        };
        return Start {
            place,
            dir: None,
            select: Vec::new(),
            connect: None,
            alone: Some(alone),
            search: false,
            tabs: None,
        };
    }
    let (place, dir, select) = if first.as_deref() == Some("--select".as_ref()) {
        let paths: Vec<PathBuf> = args.map(PathBuf::from).collect();
        let first = paths.first().filter(|p| p.is_absolute());
        match first.and_then(|p| p.parent()).map(Path::to_path_buf) {
            Some(dir) => {
                let select = paths
                    .iter()
                    .filter(|p| p.parent() == Some(dir.as_path()))
                    .cloned()
                    .collect();
                (Place::from_path(&dir), Some(dir), select)
            }
            None => (Place::Home, None, Vec::new()),
        }
    } else {
        let read = first.and_then(|arg| match arg.to_str() {
            Some(s) => Place::read_address(s, None, &[]),
            None => {
                let p = PathBuf::from(arg);
                p.is_absolute().then(|| (Place::from_path(&p), Some(p)))
            }
        });
        let (place, dir) = read.unwrap_or((Place::Home, None));
        (place, dir, Vec::new())
    };
    Start {
        place,
        dir,
        select,
        connect: None,
        alone: None,
        search: false,
        tabs: None,
    }
}

/// The PLACE [`parse_args`] opens `p` again from: a network server opens the Network list, as
/// connecting at login could ask for a password. None for a folder whose path is not UTF-8.
pub(crate) fn reopen_arg(p: &Place) -> Option<String> {
    match p {
        Place::Computer => Some("Computer".into()),
        Place::RecycleBin => Some("Recycle Bin".into()),
        Place::Network(_) => Some("Network".into()),
        _ => p.path()?.into_os_string().into_string().ok(),
    }
}

/// Runs the program; `args` are the command line after the program's name.
pub fn main(args: Vec<std::ffi::OsString>) {
    reprisebus::journal::init_logger(reprisebus::journal::PROGRAM_FILTER);
    if args
        .first()
        .is_some_and(|a| a == reprisefs::thumbpipe::FLAG)
    {
        if let Err(e) = thumbs::serve_stdio() {
            log::warn!("thumbnailer: {e}");
        }
        return;
    }
    if args.first().is_some_and(|a| a == thumbs::DECODE_FLAG) {
        if let Err(e) = thumbs::serve_decode(&args[1..]) {
            log::warn!("JPEG XL or AVIF thumbnail: {e}");
            std::process::exit(1);
        }
        return;
    }
    if args.first().is_some_and(|a| a == preview::FONT_SAMPLE) {
        if let Err(e) = preview::serve_font_sample(&args[1..]) {
            log::warn!("font sample: {e}");
            std::process::exit(1);
        }
        return;
    }
    if args.first().is_some_and(|a| a == filemanager::FLAG) {
        std::process::exit(filemanager::serve());
    }
    reprisefs::sendto::follow_phones();
    repriseapp::run(Box::new(Lazy {
        start: parse_args(args),
        inner: None,
        units: repriseapp::start_forms().text,
    }))
    .unwrap_or_else(|e| e.exit());
    // A lone Empty Recycle Bin ends as it empties, before its sound has played.
    repriseaudio::sounds::wait_quiet(std::time::Duration::from_secs(3));
}

/// A folder itself, or the folder a symlink to one points at; None for anything else.
#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
