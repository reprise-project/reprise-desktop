//! Desktop icons on the first output: one scene buffer per entry of the desktop
//! directory and each system icon, click and marquee selection, double-click
//! opening, the desktop's and icons' right-click menus, the drop menu, and an
//! inotify watch that rescans on change. Dragging icons is `dragview`'s, other
//! programs' drops are `dnddrop`'s; rename, the file clipboard and icon keys are
//! `iconedit`'s; Delete, Properties, Open With and Empty Recycle Bin are
//! `desktopfiles`'. Send To lists its destinations as the submenu opens,
//! from what the desktop read and the drive stats found (`sendto::Drives::send_to_rows`).

mod commands;
mod drives;
mod layout;
mod menus;
mod scan;
mod select;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};

use reprisefs::{apps, places, sendto};
use reprisekit::pointer::DoubleClick;
use reprisekit::selection::Selection;
use reprisescene::{BufferId, Input, TreeId};
use smithay::{
    output::Output,
    reexports::calloop::{Interest, Mode, PostAction, RegistrationToken, generic::Generic},
};

use crate::{
    compositor::{SceneOwner, State},
    desktop::{self, DesktopEntry},
    dirwatch::DirWatch,
    raster::Rect,
    textedit::TextEdit,
    theme::Theme,
};

pub(crate) use drives::DriveBins;
use layout::CellKey;
pub(crate) use menus::entry;
pub use menus::{
    IconMenu, NEW_MENU_ICONS, arrange_menu, desktop_menu, drop_menu, icon_menu, new_menu,
    open_with_menu, send_to_menu,
};
#[cfg(test)]
pub use menus::{bin_menu, place_menu};
pub use select::aimed_in;

pub struct DesktopView {
    pub dir: PathBuf,
    pub entries: Vec<DesktopEntry>,
    pub cells: Vec<Rect>,
    /// Lives in `layers.background`, above the wallpaper's buffers.
    tree: TreeId,
    nodes: Vec<BufferId>,
    /// Which entries are selected, with the focus ring's index; the model Explorer's
    /// view holds.
    pub selection: Selection,
    /// What the left button, held since `desktop_press`, is doing: a press lands
    /// either on an icon or on the desktop.
    pub press: Option<Press>,
    band_nodes: [BufferId; 4],
    /// The marquee under a look with a selection box (Glass): its fill and four
    /// lines as rects, made on the first band that needs them.
    band_box: Option<[reprisescene::RectId; 5]>,
    /// What the band nodes' strips were drawn for: scale, ink, and the across and
    /// down lengths in logical px; `draw_band` redraws only when this changes.
    band_strip: Option<(i32, reprisekit::theme::Color, i32, i32)>,
    pub drop_hilite: Option<usize>,
    /// Whether the selection shows its dotted focus ring: hidden until the keyboard
    /// is used on the desktop.
    pub(crate) focus_ring: bool,
    /// Pairs presses on one icon, or on empty desktop (None).
    last_click: DoubleClick<Option<usize>>,
    /// An in-progress inline rename: the icon index and the edit.
    pub rename: Option<(usize, TextEdit)>,
    /// Left presses in the rename box, paired into the double click that selects a word.
    pub(crate) rename_clicks: DoubleClick<()>,
    /// Where a drag resting on the rename box would drop its text (`dialogdrop`): the box
    /// shows its caret there, without the selection, until the drag goes.
    pub rename_drop: Option<usize>,
    watch: Option<DirWatch>,
    /// The folders a file job is adding to `watch`. One on a drive that never answers keeps its
    /// job, and no second one starts.
    watch_adds: HashSet<PathBuf>,
    /// Re-armed on every inotify wakeup; the rescan runs when it fires, so an unpack
    /// costs one scan, not one per file.
    settle: Option<RegistrationToken>,
    /// What each node was last drawn from, so `relayout_desktop` redraws only changed
    /// cells.
    drawn: Vec<Option<CellKey<'static>>>,
    /// The config theme the cells were drawn from and its scaled copy; a change to
    /// either clears `drawn`.
    drawn_theme: Option<(Theme, Theme)>,
    pub positions: BTreeMap<String, (i32, i32)>,
    pub auto_arrange: bool,
    pub(crate) system_names: BTreeMap<String, String>,
    pub(crate) mime: std::sync::Arc<apps::MimeDb>,
    /// A rescan is reading the folder off the loop; `rescan_again` asks for one more after it.
    scanning: bool,
    rescan_again: bool,
    /// The icon size the last read decoded the icon files at, and the icon cache's
    /// `file_drops` after it put them in: a change in either means a read again.
    icons_read: (i32, u64),
    /// Cells answers pinned since the last read finished, which `cells` does not show yet; 100%
    /// cells ([`desktop::to_saved`]), so a text size change keeps them.
    landed: Vec<(i32, i32)>,
    pub(crate) send_to: Vec<reprisefs::sendto::SendToRow>,
    pub(crate) send_dirs: sendto::Dirs,
    pub(crate) drives: sendto::Drives,
    pub(crate) bins: DriveBins,
}

/// A rubber-band selection in progress: the press point, the selection as it
/// stood then (what a Ctrl-drag toggles against) and the band once the pointer
/// has travelled `desktop::DRAG_PX`, in layout px.
#[derive(Clone, Debug)]
pub struct Marquee {
    pub origin: (i32, i32),
    pub base: BTreeSet<usize>,
    pub ctrl: bool,
    pub band: Option<Rect>,
}

#[derive(Clone, Debug)]
pub enum Press {
    /// A plain press on an icon of a multi-selection: the set stays for a drag of
    /// all of them, and collapses to this icon only if the button comes up without
    /// one (`settle_click`).
    Icon(usize),
    Marquee(Marquee),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesktopCommand {
    Refresh,
    NextBackground,
    NewFolder,
    NewTextDocument,
    Settings,
    Open,
    Trash,
    Rename,
    AllowLaunching,
    CreateShortcut,
    Explore,
    OpenFileLocation,
    OpenWith,
    SendTo,
    SendToDest(usize),
    IconProperties,
    RepriseSettings,
    AboutReprise,
    Print,
    EmptyBin,
    BinProperties,
    Edit,
    OpenWithApp(String),
    ChooseProgram,
    Cut,
    Copy,
    Paste,
    PasteShortcut,
    PasteInto,
    /// Paste on a launcher: the clipboard's files open in its program.
    PasteOpen,
    ArrangeIcons,
    New,
    ArrangeByName,
    AutoArrange,
    DropCopy,
    DropMove,
    DropShortcut,
    DropCancel,
}

impl DesktopCommand {
    pub fn opens_submenu(&self) -> bool {
        matches!(
            self,
            DesktopCommand::ArrangeIcons
                | DesktopCommand::New
                | DesktopCommand::OpenWith
                | DesktopCommand::SendTo
        )
    }
}

/// Create the desktop's tree and watch, then list the icons. Runs once at
/// startup, before the outputs come; does nothing without a home directory.
pub fn setup(state: &mut State) {
    let Some(dir) = desktop::desktop_dir() else {
        log::info!("desktop icons: no home directory; disabled");
        return;
    };
    let persisted = crate::state::load();
    let tree = state.scene.tree_create(state.layers.background);
    state.scene.set_data(tree, Some(SceneOwner::Desktop));
    let band_nodes = std::array::from_fn(|_| {
        let n = state.scene.buffer_create(tree);
        state.scene.set_input(n, Input::Passthrough);
        state.scene.set_enabled(n, false);
        n
    });
    // The watches themselves are added by the rescan, and again whenever a
    // watched folder is deleted and made anew.
    let watch = match DirWatch::new() {
        Ok(w) => {
            watch_on_loop(state, &w);
            mounts_on_loop(state);
            gvfs_on_loop(state);
            Some(w)
        }
        Err(e) => {
            log::warn!("desktop icons: inotify unavailable: {e}");
            None
        }
    };
    state.desktop = Some(DesktopView {
        dir,
        entries: Vec::new(),
        cells: Vec::new(),
        tree,
        nodes: Vec::new(),
        selection: Selection::default(),
        press: None,
        band_nodes,
        band_box: None,
        band_strip: None,
        drop_hilite: None,
        focus_ring: false,
        last_click: DoubleClick::default(),
        rename: None,
        rename_clicks: DoubleClick::default(),
        rename_drop: None,
        watch,
        watch_adds: HashSet::new(),
        settle: None,
        drawn: Vec::new(),
        drawn_theme: None,
        positions: persisted.desktop_positions,
        auto_arrange: persisted.auto_arrange,
        system_names: persisted.system_icon_names,
        mime: std::sync::Arc::new(apps::mime_globs()),
        scanning: false,
        rescan_again: false,
        icons_read: (0, 0),
        landed: Vec::new(),
        send_to: Vec::new(),
        send_dirs: sendto::Dirs::default(),
        drives: sendto::Drives::new(places::MountedDrives::new()),
        bins: DriveBins::default(),
    });
    state.rescan_desktop();
    state.scan_drives();
}

/// Poll `watch`'s fd on the loop through a copy of it, which shares the queue.
fn watch_on_loop(state: &mut State, watch: &DirWatch) {
    // SAFETY: the fd is the watch's inotify fd, open for this call.
    let fd = match unsafe { BorrowedFd::borrow_raw(watch.fd()) }.try_clone_to_owned() {
        Ok(fd) => fd,
        Err(e) => return log::warn!("desktop icons: no watch fd: {e}"),
    };
    let source = Generic::new(fd, Interest::READ, Mode::Level);
    let added = state.handle.insert_source(source, |_, _, state| {
        state.desktop_watch_ready();
        Ok(PostAction::Continue)
    });
    if let Err(e) = added {
        log::warn!("desktop icons: {}", e.error);
    }
}

/// Rescan the desktop and list each Browse dialog again when gvfs metadata changes for a file
/// in the folder it shows: a folder icon Nautilus or `gio set` sets.
fn gvfs_on_loop(state: &mut State) {
    let (ping, source) = match smithay::reexports::calloop::ping::make_ping() {
        Ok(p) => p,
        Err(e) => return log::warn!("desktop icons: no gvfs ping: {e}"),
    };
    let changes = reprisefs::gvfsmeta::Changes::new(move || ping.ping());
    let added = state.handle.insert_source(source, move |_, _, state| {
        let folders: BTreeSet<PathBuf> = (changes.take().iter())
            .filter_map(|f| f.parent().map(Path::to_path_buf))
            .collect();
        if (state.desktop.as_ref()).is_some_and(|v| folders.contains(&v.dir)) {
            state.settle_rescan();
        }
        state.relist_pickers(&folders);
    });
    if let Err(e) = added {
        log::warn!("desktop icons: {}", e.error);
    }
}

/// Rescan on each mount or unmount, so a drive mounted later has its trash
/// watched. `/proc/self/mounts` wakes its pollers on every mount change and is
/// always readable, so only edge mode tells the changes apart.
fn mounts_on_loop(state: &mut State) {
    let file = match std::fs::File::open("/proc/self/mounts") {
        Ok(f) => f,
        Err(e) => return log::warn!("desktop icons: no mount table: {e}"),
    };
    let source = Generic::new(file, Interest::READ, Mode::Edge);
    let added = state.handle.insert_source(source, |_, _, state| {
        state.scan_drives();
        state.settle_rescan();
        Ok(PostAction::Continue)
    });
    if let Err(e) = added {
        log::warn!("desktop icons: {}", e.error);
    }
}

/// Lists the desktop and each Browse dialog again when the folder it shows listed a link
/// unfollowed whose target has since answered.
pub fn relist_on_loop(state: &mut State) {
    let (ping, source) = match smithay::reexports::calloop::ping::make_ping() {
        Ok(p) => p,
        Err(e) => return log::warn!("desktop icons: no relist ping: {e}"),
    };
    let relist = reprisefs::listing::Relist::new(move || ping.ping());
    let added = state.handle.insert_source(source, move |_, _, state| {
        let due = relist.take();
        if (state.desktop.as_ref()).is_some_and(|v| due.contains(&v.dir)) {
            state.settle_rescan();
        }
        state.relist_pickers(&due);
    });
    if let Err(e) = added {
        log::warn!("desktop icons: {}", e.error);
    }
}

impl State {
    /// The primary output: the one `[[outputs]] primary` names while in the layout,
    /// else the first with a non-empty layout box. Icons sit on it; balloons and the
    /// media panel show there.
    pub(crate) fn first_output(&self) -> Option<Output> {
        let shown = |o: &&Output| self.layout_box(Some(o)).width > 0;
        let outputs = self.scene.outputs();
        let primary = self.config.primary_output_name();
        outputs
            .iter()
            .filter(shown)
            .find(|o| primary == Some(o.name().as_str()))
            .or_else(|| outputs.iter().find(shown))
            .cloned()
    }
}
