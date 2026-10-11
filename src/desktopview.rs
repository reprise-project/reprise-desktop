//! Desktop icons on the first output: one scene buffer per entry of the desktop
//! directory and each system icon, click and marquee selection, double-click
//! opening, the desktop's and icons' right-click menus, the drop menu, and an
//! inotify watch that rescans on change. Dragging icons is `dragview`'s, other
//! programs' drops are `dnddrop`'s; rename, the file clipboard and icon keys are
//! `iconedit`'s; Delete, Properties, Open With and Empty Recycle Bin are
//! `desktopfiles`'. Send To lists its destinations as the submenu opens
//! (`reprisefs::sendto::session_rows`).

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::time::Duration;

use inotify::WatchMask;
use reprisefs::{apps, names, ops, places};
use reprisekit::selection::{self, Mods, Selection};
use reprisescene::{BufferId, Input, TreeId};
use smithay::{
    output::Output,
    reexports::calloop::{
        Interest, Mode, PostAction, RegistrationToken,
        generic::Generic,
        timer::{TimeoutAction, Timer},
    },
    utils::Rectangle,
};

use crate::{
    compositor::{SceneOwner, State},
    desktop::{self, DesktopEntry, ICON_PX, IconId, Target},
    dirwatch::{DirWatch, Want},
    output::upload_scaled,
    raster::Rect,
    startmenu::MenuItem,
    text::split_mnemonic,
    textedit::TextEdit,
    theme::Theme,
};

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
    last_click: Option<(u32, usize)>,
    /// An in-progress inline rename: the icon index and the edit.
    pub rename: Option<(usize, TextEdit)>,
    pub(crate) rename_press: Option<u32>,
    /// Where a drag resting on the rename box would drop its text (`dialogdrop`): the box
    /// shows its caret there, without the selection, until the drag goes.
    pub rename_drop: Option<usize>,
    watch: Option<DirWatch>,
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
    pub(crate) mime: apps::MimeDb,
    pub(crate) send_to: Vec<reprisefs::sendto::SendToRow>,
    pub(crate) drives: places::MountedDrives,
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
    ArrangeIcons,
    New,
    ArrangeByName,
    AutoArrange,
    DropCopy,
    DropMove,
    DropShortcut,
    DropCancel,
}

fn entry(label: &str) -> MenuItem {
    let (label, mnemonic) = split_mnemonic(label);
    MenuItem::entry(label).mnemonic(mnemonic)
}

fn disabled_row(label: &str) -> MenuItem {
    entry(label).enabled(false)
}

/// A menu's rows, each with what it does, so a row and its command cannot drift.
type Rows = Vec<(MenuItem, Option<DesktopCommand>)>;

const SEP: (MenuItem, Option<DesktopCommand>) = (MenuItem::Separator, None);

fn split(rows: Rows) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    rows.into_iter().unzip()
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

/// The empty-desktop menu, ending on Reprise Settings and About Reprise. Paste
/// and Paste Shortcut are greyed unless `can_paste`. With `show_files` false, New
/// and the Paste pair are greyed (what they make would land in ~/Desktop, which
/// is not shown). While a `slideshow` runs, "Next desktop background" follows
/// Refresh.
pub fn desktop_menu(
    can_paste: bool,
    show_files: bool,
    slideshow: bool,
) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    use DesktopCommand::*;
    let new = if show_files {
        (entry("Ne&w").submenu(true), Some(New))
    } else {
        (disabled_row("Ne&w").submenu(true), None)
    };
    let paste = |label: &str, cmd: DesktopCommand| {
        if can_paste && show_files {
            (entry(label), Some(cmd))
        } else {
            (disabled_row(label), None)
        }
    };
    let mut rows = vec![
        (entry("Arrange &Icons By").submenu(true), Some(ArrangeIcons)),
        (entry("R&efresh"), Some(Refresh)),
        SEP,
        paste("&Paste", Paste),
        paste("Paste &Shortcut", PasteShortcut),
        SEP,
        new,
        SEP,
        (entry("&Reprise Settings"), Some(Settings)),
        (entry("&About Reprise"), Some(AboutReprise)),
    ];
    if slideshow {
        rows.insert(2, (entry("Next desktop &background"), Some(NextBackground)));
    }
    split(rows)
}

pub fn new_menu() -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    use DesktopCommand::*;
    split(vec![
        (entry("&Folder"), Some(NewFolder)),
        SEP,
        (entry("&Text Document"), Some(NewTextDocument)),
    ])
}

pub const NEW_MENU_ICONS: [Option<&str>; 3] = [Some("folder"), None, Some("text-x-generic")];

pub fn arrange_menu(auto_arrange: bool) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    use DesktopCommand::*;
    split(vec![
        (entry("&Name"), Some(ArrangeByName)),
        SEP,
        (
            entry("&Auto Arrange").checked(auto_arrange),
            Some(AutoArrange),
        ),
    ])
}

/// The menu a right-button drop opens, `default`'s row in bold (what a
/// left drop would have done, or Create Shortcuts Here for a shortcut
/// drag from another program). A drag carrying a `system` icon has no
/// Copy Here: its place is not a file to copy. Rows another program's
/// drag does not allow (`allowed`) are greyed.
pub fn drop_menu(
    system: bool,
    default: crate::dragview::DropOp,
    allowed: crate::dragview::Allowed,
) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    use crate::dragview::DropOp;
    use DesktopCommand::*;
    // A system icon is never copied: its default falls to Move.
    let default = match default {
        DropOp::Copy if system => DropOp::Move,
        op => op,
    };
    let mut rows = Rows::new();
    if !system {
        rows.push((
            entry("&Copy Here")
                .bold(default == DropOp::Copy)
                .enabled(allowed.copy),
            Some(DropCopy),
        ));
    }
    rows.extend([
        (
            entry("&Move Here")
                .bold(default == DropOp::Move)
                .enabled(allowed.moves),
            Some(DropMove),
        ),
        (
            entry("Create &Shortcuts Here")
                .bold(default == DropOp::Shortcut)
                .enabled(allowed.copy),
            Some(DropShortcut),
        ),
        SEP,
        (entry("Cancel"), Some(DropCancel)),
    ]);
    split(rows)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IconMenu {
    pub is_dir: bool,
    pub print: bool,
    pub edit: bool,
    pub allow_launching: bool,
    pub launcher: bool,
    pub in_set: bool,
}

/// A file's or folder's context menu. A file gets its verbs (`print`, `edit`) and
/// Open With, and Allow Launching with `allow_launching`. A launcher gets a greyed
/// Run as... under Open: a program runs in the session of the logged-in user and
/// another user's program cannot reach its display. A folder gets Explore, the
/// greyed Sharing and Security... (no sheet behind it), and Paste under Copy. On
/// an icon `in_set` of a multi-selection it is the same menu less Sharing and
/// Security..., its rows acting on the whole set but for Explore, Paste, Rename
/// and Properties.
pub fn icon_menu(m: IconMenu) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    use DesktopCommand::*;
    let mut rows = vec![(entry("&Open").bold(true), Some(Open))];
    if m.is_dir {
        rows.push((entry("E&xplore"), Some(Explore)));
        if !m.in_set {
            rows.extend([SEP, (disabled_row("S&haring and Security..."), None)]);
        }
    } else {
        if m.launcher {
            rows.push((disabled_row("Run &as..."), None));
        }
        // The file verbs between Open and Open With: Print, then Edit.
        if m.print {
            rows.push((entry("&Print"), Some(Print)));
        }
        if m.edit {
            rows.push((entry("&Edit"), Some(Edit)));
        }
        rows.push((entry("Open Wit&h").submenu(true), Some(OpenWith)));
        if m.allow_launching {
            rows.push((entry("Allow &Launching"), Some(AllowLaunching)));
        }
    }
    rows.extend([
        SEP,
        (entry("Se&nd To").submenu(true), Some(SendTo)),
        SEP,
        (entry("Cu&t"), Some(Cut)),
        (entry("&Copy"), Some(Copy)),
    ]);
    if m.is_dir {
        rows.push((entry("&Paste"), Some(PasteInto)));
    }
    rows.extend([
        SEP,
        (entry("Create &Shortcut"), Some(CreateShortcut)),
        (entry("&Delete"), Some(Trash)),
        (entry("Rena&me"), Some(Rename)),
        SEP,
        (entry("P&roperties"), Some(IconProperties)),
    ]);
    split(rows)
}

pub fn send_to_menu(
    rows: &[reprisefs::sendto::SendToRow],
) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    rows.iter()
        .enumerate()
        .map(|(i, r)| {
            let label = r.label.replace('&', "&&");
            if r.enabled {
                (entry(&label), Some(DesktopCommand::SendToDest(i)))
            } else {
                (disabled_row(&label), None)
            }
        })
        .unzip()
}

/// The "Open With" submenu: one row per program in `apps` (desktop-file id and
/// name), then "Choose Program..." after a separator (alone when there are none).
pub fn open_with_menu(apps: &[(String, String)]) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    let mut rows: Rows = apps
        .iter()
        .map(|(id, name)| {
            (
                entry(&name.replace('&', "&&")),
                Some(DesktopCommand::OpenWithApp(id.clone())),
            )
        })
        .collect();
    if !apps.is_empty() {
        rows.push(SEP);
    }
    rows.push((
        entry("&Choose Program..."),
        Some(DesktopCommand::ChooseProgram),
    ));
    split(rows)
}

/// A place icon's menu. Documents and Home (real folders) open, explore and
/// have Copy, and Properties with `sheet` (a folder that exists); Computer opens
/// and ends on Reprise Settings and About Reprise, after a separator. All three
/// Create Shortcut, Delete (hide) and Rename.
pub fn place_menu(
    place: desktop::Place,
    sheet: bool,
) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    use DesktopCommand::*;
    let computer = place == desktop::Place::Computer;
    let mut rows = vec![(entry("&Open").bold(true), Some(Open))];
    if !computer {
        rows.extend([
            (entry("E&xplore"), Some(Explore)),
            SEP,
            (entry("&Copy"), Some(Copy)),
        ]);
    }
    rows.extend([
        SEP,
        (entry("Create &Shortcut"), Some(CreateShortcut)),
        (entry("&Delete"), Some(Trash)),
        (entry("Rena&me"), Some(Rename)),
    ]);
    if computer {
        rows.extend([
            SEP,
            (entry("&Reprise Settings"), Some(RepriseSettings)),
            (entry("&About Reprise"), Some(AboutReprise)),
        ]);
    }
    if sheet {
        rows.extend([SEP, (entry("P&roperties"), Some(IconProperties))]);
    }
    split(rows)
}

pub fn bin_menu(empty: bool) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    use DesktopCommand::*;
    let empty_bin = if empty {
        (disabled_row("Empt&y Recycle Bin"), None)
    } else {
        (entry("Empt&y Recycle Bin"), Some(EmptyBin))
    };
    split(vec![
        (entry("&Open").bold(true), Some(Open)),
        (entry("E&xplore"), Some(Explore)),
        empty_bin,
        SEP,
        (entry("Create &Shortcut"), Some(CreateShortcut)),
        SEP,
        (entry("P&roperties"), Some(BinProperties)),
    ])
}

/// The indices of `set` a command aimed at `index` takes: `index` first, then
/// the rest of the set; with `files_only`, only its files (a place or the Recycle
/// Bin is no file to copy or send). Indices past `entries` are dropped.
pub fn aimed_in(
    entries: &[DesktopEntry],
    set: &[usize],
    index: usize,
    files_only: bool,
) -> Vec<usize> {
    std::iter::once(index)
        .chain(set.iter().copied().filter(|&i| i != index))
        .filter(|&i| {
            entries
                .get(i)
                .is_some_and(|e| i == index || !files_only || e.kind == desktop::Kind::File)
        })
        .collect()
}

fn source_of(e: &DesktopEntry) -> crate::dragview::DragSource {
    crate::dragview::DragSource::Icon {
        id: e.id(),
        path: e.path.clone(),
        name: e.shortcut_base().to_string(),
    }
}

/// A 1 px dotted line `len` logical px long, across or `vertical`, with the
/// screen-wide pattern of a line starting at the origin: a dot on every odd px.
fn band_strip(
    len: i32,
    vertical: bool,
    scale: i32,
    ink: reprisekit::theme::Color,
) -> reprisekit::raster::Canvas {
    let (w, h) = if vertical { (1, len) } else { (len, 1) };
    let mut c = reprisekit::raster::Canvas::new((w * scale) as u32, (h * scale) as u32);
    reprisekit::classic::widgets::draw_focus_ring_screen(
        &mut c,
        Rect::new(0, 0, w * scale, h * scale),
        scale,
        ink,
        (0, 0),
    );
    c
}

/// The part of a `band_strip`, in buffer px, that draws `edge`: it starts
/// `(x + y) % 2` px in, so the edge's dots fall where x + y is odd.
fn band_crop(edge: Rect, vertical: bool, scale: i32) -> Rect {
    let phase = (edge.x + edge.y).rem_euclid(2) * scale;
    if vertical {
        Rect::new(0, phase, scale, edge.height * scale)
    } else {
        Rect::new(phase, 0, edge.width * scale, scale)
    }
}

const SETTLE_MS: u64 = 100;

/// The icons the desktop lists: the system icons in `system_icons` order, then
/// the desktop directory's files when `show_files`. `system` makes one system
/// icon by config name, None to leave it out; `files` lists the directory and
/// runs only when the files show.
fn listed(
    system_icons: &[String],
    show_files: bool,
    system: impl FnMut(&str) -> Option<DesktopEntry>,
    files: impl FnOnce() -> Vec<DesktopEntry>,
) -> Vec<DesktopEntry> {
    let mut entries: Vec<DesktopEntry> = system_icons
        .iter()
        .map(String::as_str)
        .filter_map(system)
        .collect();
    if show_files {
        entries.extend(files());
    }
    entries
}

fn migrate_launcher_keys(
    entries: &[DesktopEntry],
    positions: &mut BTreeMap<String, (i32, i32)>,
) -> bool {
    let mut moved = false;
    for e in entries {
        if !matches!(e.target, Target::Launch(_))
            || positions.contains_key(&e.name)
            || entries.iter().any(|o| o.name == e.label)
        {
            continue;
        }
        if let Some(cell) = positions.remove(&e.label) {
            positions.insert(e.name.clone(), cell);
            moved = true;
        }
    }
    moved
}

/// What the desktop watch covers. When `files` shows the folder's files: the
/// desktop folder (ATTRIB, because chmod +x turns a `.desktop` file into a
/// launcher) and its parent (for the folder being made). The home trash and its
/// files folder once they exist, and each mounted drive's trash, so the Recycle
/// Bin's icon flips between empty and full.
fn watch_wants(dir: &Path, files: bool, system_icons: &[String]) -> Vec<Want> {
    let mut want = Vec::new();
    if files {
        want.push(Want {
            dir: dir.to_path_buf(),
            mask: WatchMask::CREATE
                | WatchMask::DELETE
                | WatchMask::MOVED_FROM
                | WatchMask::MOVED_TO
                | WatchMask::CLOSE_WRITE
                | WatchMask::ATTRIB,
            only: Vec::new(),
        });
        if let (Some(parent), Some(name)) = (dir.parent(), dir.file_name()) {
            want.push(Want {
                dir: parent.to_path_buf(),
                mask: WatchMask::CREATE | WatchMask::MOVED_TO,
                only: vec![name.to_os_string()],
            });
        }
    }
    if system_icons.iter().any(|n| n == "recycle-bin")
        && let Some(files) = places::trash_dir()
    {
        let trash =
            WatchMask::CREATE | WatchMask::DELETE | WatchMask::MOVED_FROM | WatchMask::MOVED_TO;
        if let Some(parent) = files.parent() {
            want.push(Want {
                dir: parent.to_path_buf(),
                mask: trash,
                only: Vec::new(),
            });
        }
        want.push(Want {
            dir: files,
            mask: trash,
            only: Vec::new(),
        });
        // Each mounted drive's trash too, and each drive's top for its
        // trash being made; an eject drops the watch, which is a change.
        let Some(bins) = reprisefs::trash::Bins::system() else {
            return want;
        };
        let names = [".Trash".into(), format!(".Trash-{}", bins.uid).into()];
        for top in bins.mounts.iter().filter(|m| !bins.unlisted.contains(*m)) {
            want.push(Want {
                dir: top.clone(),
                mask: WatchMask::CREATE | WatchMask::MOVED_TO,
                only: names.to_vec(),
            });
        }
        for root in bins.roots().into_iter().skip(1) {
            want.push(Want {
                dir: root.join("files"),
                mask: trash,
                only: Vec::new(),
            });
            want.push(Want {
                dir: root,
                mask: trash,
                only: Vec::new(),
            });
        }
    }
    want
}

/// Where each icon of `old` sits in `new`, matched by `DesktopEntry::id`; None
/// for one that is gone. Files are looked up by path in one map, so a rescan with
/// everything selected stays linear.
fn remap(old: &[DesktopEntry], new: &[DesktopEntry]) -> Vec<Option<usize>> {
    // Reversed, so a path listed twice maps to its first index, as `find` does.
    let files: std::collections::HashMap<&Path, usize> = new
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, e)| e.kind == desktop::Kind::File)
        .map(|(i, e)| (e.path.as_path(), i))
        .collect();
    old.iter()
        .map(|e| match e.kind {
            desktop::Kind::File => files.get(e.path.as_path()).copied(),
            _ => desktop::find(new, &e.id()),
        })
        .collect()
}

/// Everything a cell's pixels depend on besides the theme (`drawn_theme`). Equal
/// keys mean equal pixels. Built borrowed and stored owned, so an unchanged cell
/// costs no allocation.
#[derive(Clone, PartialEq, Eq)]
struct CellKey<'a> {
    label: Cow<'a, str>,
    icon: Cow<'a, str>,
    shortcut: bool,
    cell: Rect,
    selected: bool,
    ring: bool,
    cut: bool,
    editing: Option<(Cow<'a, str>, std::ops::Range<usize>, usize, bool)>,
    preedit: Option<reprisekit::textedit::Preedit>,
}

impl CellKey<'_> {
    fn into_owned(self) -> CellKey<'static> {
        CellKey {
            label: Cow::Owned(self.label.into_owned()),
            icon: Cow::Owned(self.icon.into_owned()),
            editing: self
                .editing
                .map(|(t, sel, caret, on)| (Cow::Owned(t.into_owned()), sel, caret, on)),
            ..self
        }
    }
}

struct Redraw {
    node: BufferId,
    cell: Rect,
    label: String,
    icon: String,
    shortcut: bool,
    cut: bool,
    selection: Option<desktop::Selection>,
    editing: Option<(TextEdit, bool)>,
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
        last_click: None,
        rename: None,
        rename_press: None,
        rename_drop: None,
        watch,
        settle: None,
        drawn: Vec::new(),
        drawn_theme: None,
        positions: persisted.desktop_positions,
        auto_arrange: persisted.auto_arrange,
        system_names: persisted.system_icon_names,
        mime: apps::mime_globs(),
        send_to: Vec::new(),
        drives: places::MountedDrives::new(),
    });
    state.rescan_desktop();
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
        state.settle_rescan();
        Ok(PostAction::Continue)
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

    fn desktop_watch_ready(&mut self) {
        let Some(view) = self.desktop.as_mut() else {
            return;
        };
        if view.watch.as_mut().is_some_and(|w| w.drain()) {
            self.settle_rescan();
        }
    }

    /// Rescan once changes stop arriving for `SETTLE_MS`.
    fn settle_rescan(&mut self) {
        let Some(view) = self.desktop.as_mut() else {
            return;
        };
        if let Some(t) = view.settle.take() {
            self.handle.remove(t);
        }
        let timer = Timer::from_duration(Duration::from_millis(SETTLE_MS));
        let token = self.handle.insert_source(timer, |_, _, state| {
            if let Some(view) = state.desktop.as_mut() {
                view.settle = None;
            }
            state.rescan_desktop();
            TimeoutAction::Drop
        });
        match token {
            Ok(t) => {
                if let Some(view) = self.desktop.as_mut() {
                    view.settle = Some(t);
                }
            }
            // Without the timer every wakeup rescans at once.
            Err(_) => self.rescan_desktop(),
        }
    }

    pub fn rescan_desktop(&mut self) {
        let State {
            desktop: Some(view),
            config,
            icons,
            ..
        } = self
        else {
            return;
        };
        // Watch before reading, so a change between the two is not missed.
        if let Some(w) = view.watch.as_mut() {
            w.sync(&watch_wants(
                &view.dir,
                config.desktop_icons,
                &config.system_icons,
            ));
        }
        let entries = listed(
            &config.system_icons,
            config.desktop_icons,
            |name| {
                if name == "recycle-bin" {
                    let files = places::trash_dir()?;
                    let empty = reprisefs::trash::Bins::system().is_none_or(|b| b.all_empty());
                    Some(desktop::recycle_bin(files, empty))
                } else {
                    let e = desktop::place_shown(name, &view.system_names);
                    if e.is_none() && desktop::Place::from_config(name).is_none() {
                        log::warn!("desktop: unknown system icon {name:?}");
                    }
                    e
                }
            },
            || desktop::scan(&view.dir, &view.mime, &|n| icons.has(n)),
        );
        let old = std::mem::replace(&mut view.entries, entries);
        let moved = remap(&old, &view.entries);
        let remap = |i: &usize| moved.get(*i).copied().flatten();
        view.selection = Selection {
            set: view.selection.set.iter().filter_map(remap).collect(),
            focus: view.selection.focus.as_ref().and_then(remap),
            anchor: None,
        };
        // The rename follows its file; it ends only when the file is gone.
        view.rename = view
            .rename
            .take()
            .and_then(|(i, edit)| Some((remap(&i)?, edit)));
        view.last_click = None;
        if migrate_launcher_keys(&view.entries, &mut view.positions) {
            crate::state::save_desktop(view.auto_arrange, &view.positions);
        }
        self.relayout_desktop();
    }

    pub fn relayout_desktop(&mut self) {
        let out = self.first_output();
        let (area, scale) = match &out {
            Some(o) => (self.usable_area(Some(o)), self.draw_scale_at(Some(o))),
            None => (Rect::default(), 1),
        };
        let caret_on = self.caret.on && self.editor() == Some(crate::caret::Editor::Rename);
        let preedit = self.ime_field_preedit(crate::caret::Editor::Rename);
        let mut spot = None;
        // A Cut file shows its icon at half strength until it is pasted.
        let cut: Vec<PathBuf> = self
            .own_file_clip()
            .filter(|c| c.cut)
            .map(|c| c.paths)
            .unwrap_or_default();
        let Some(mut view) = self.desktop.take() else {
            return;
        };
        view.cells = if out.is_none() {
            Vec::new()
        } else if view.auto_arrange {
            desktop::layout(
                desktop::Cell::square(desktop::CELL),
                area,
                view.entries.len(),
            )
        } else {
            let names: Vec<&str> = view.entries.iter().map(|e| e.name.as_str()).collect();
            desktop::layout_free(area, &names, &view.positions)
        };
        let count = view.cells.len();
        let mut changed = view.nodes.len() != count;
        while view.nodes.len() > count {
            if let Some(n) = view.nodes.pop() {
                self.scene.destroy(n);
            }
        }
        while view.nodes.len() < count {
            view.nodes.push(self.scene.buffer_create(view.tree));
        }
        // The labels stay at the dialogs' text size: the grid's cells are
        // fixed pixels, and a larger label would run into the next icon.
        let base = &self.config.dialog_theme;
        if view
            .drawn_theme
            .as_ref()
            .is_none_or(|(b, t)| b != base || t.scale != scale.max(1))
        {
            view.drawn_theme = Some((base.clone(), base.scaled(scale)));
            view.drawn.clear();
        }
        view.drawn.resize(count, None);
        let mut redraw = Vec::new();
        let dropping = view
            .rename
            .as_ref()
            .zip(view.rename_drop)
            .map(|((_, e), at)| {
                let mut e = e.clone();
                e.place(at, false);
                e
            });
        for (i, (e, cell)) in view.entries.iter().zip(&view.cells).enumerate() {
            let is_cut = cut.contains(&e.path);
            let editing = view
                .rename
                .as_ref()
                .filter(|(ri, _)| *ri == i)
                .map(|(_, edit)| match &dropping {
                    Some(d) => (d, true),
                    None => (edit, caret_on),
                });
            let selected = view.selection.is(i) || view.drop_hilite == Some(i);
            let ring = view.focus_ring && view.selection.focus == Some(i);
            let key = CellKey {
                label: Cow::Borrowed(&e.label),
                icon: Cow::Borrowed(&e.icon),
                shortcut: e.shortcut,
                cell: *cell,
                selected,
                ring,
                cut: is_cut,
                editing: editing.map(|(ed, on)| {
                    (
                        Cow::Borrowed(ed.text.as_str()),
                        ed.selection(),
                        ed.caret(),
                        on,
                    )
                }),
                preedit: editing.and(preedit.clone()),
            };
            if view.drawn[i].as_ref() == Some(&key) {
                continue;
            }
            view.drawn[i] = Some(key.into_owned());
            redraw.push(Redraw {
                node: view.nodes[i],
                cell: *cell,
                label: e.label.clone(),
                icon: e.icon.clone(),
                shortcut: e.shortcut,
                cut: is_cut,
                selection: selected.then_some(desktop::Selection {
                    ring,
                    origin: (cell.x, cell.y),
                }),
                editing: editing.map(|(ed, on)| (ed.clone(), on)),
            });
        }
        if let Some((_, theme)) = view.drawn_theme.as_ref() {
            for r in &redraw {
                let mut icon = self.icons.get_shared(&r.icon, ICON_PX * scale);
                if r.cut
                    && let Some(ic) = icon.as_mut()
                {
                    std::sync::Arc::make_mut(ic).fade(1, 2);
                }
                let editing = r
                    .editing
                    .as_ref()
                    .map(|(ed, on)| (ed, reprisekit::textedit::Caret::new(true, *on)));
                let mut canvas = desktop::draw_cell(
                    theme,
                    &self.text,
                    &r.label,
                    icon.as_deref(),
                    r.shortcut,
                    r.selection,
                    editing,
                    editing.and(preedit.as_ref()),
                );
                // A rename box wider than the cell comes back on a wider
                // canvas centred on the cell; it goes on top of its
                // neighbours.
                let extra =
                    (canvas.width as i32 - ICON_PX.max(desktop::CELL) * scale) / (2 * scale.max(1));
                self.scene.set_position(r.node, r.cell.x - extra, r.cell.y);
                if editing.is_some() {
                    self.scene.raise_to_top(r.node);
                    let origin = (r.cell.x - extra, r.cell.y);
                    spot = Some((canvas.take_spot(), origin));
                }
                upload_scaled(&mut self.scene, r.node, &canvas, scale);
            }
        }
        changed |= !redraw.is_empty();
        self.desktop = Some(view);
        if let Some((spot, origin)) = spot {
            self.ime_field_drawn(crate::caret::Editor::Rename, spot, origin, scale);
        }
        if changed {
            self.redraw_all();
        }
        self.water_obstacles();
    }

    /// Forget what the cells were drawn from, so the next relayout redraws them all
    /// (a new icon cache or text renderer, which `CellKey` cannot see).
    pub fn invalidate_desktop(&mut self) {
        if let Some(view) = self.desktop.as_mut() {
            view.drawn.clear();
        }
    }

    pub fn desktop_hit(&self, lx: f64, ly: f64) -> Option<usize> {
        let view = self.desktop.as_ref()?;
        let (x, y) = (lx.floor() as i32, ly.floor() as i32);
        view.cells
            .iter()
            .position(|c| c.contains(x, y))
            .filter(|&i| i < view.entries.len())
    }

    pub(crate) fn desktop_area(&self) -> Rect {
        let out = self.first_output();
        self.usable_area(out.as_ref())
    }

    /// A left press on the desktop at `at`: on a cell it clicks it (Ctrl toggles,
    /// Shift ranges, see `Selection::click`), and a second plain press on the same
    /// cell within `double_click_ms` opens it. A plain press on an icon of a
    /// multi-selection keeps the set for a drag (`Press::Icon`). On empty desktop it
    /// clears the selection (Ctrl keeps it) and arms the marquee at `at`.
    pub fn desktop_press(
        &mut self,
        index: Option<usize>,
        at: (i32, i32),
        time_msec: u32,
        mods: Mods,
    ) {
        let threshold = self.config.theme.metrics.double_click_ms;
        let Some(view) = self.desktop.as_mut() else {
            return;
        };
        let before = view.selection.clone();
        let mut open = None;
        match index.filter(|&i| i < view.entries.len()) {
            Some(i) => {
                let double = desktop::is_double_click(view.last_click, time_msec, i, threshold);
                view.last_click = if double { None } else { Some((time_msec, i)) };
                let in_set =
                    mods == Mods::default() && view.selection.len() > 1 && view.selection.is(i);
                view.press = in_set.then_some(Press::Icon(i));
                if !in_set {
                    view.selection.click(i, mods);
                }
                if double && mods == Mods::default() {
                    open = Some(i);
                }
            }
            None => {
                view.last_click = None;
                if !mods.ctrl {
                    view.selection.clear();
                }
                view.press = Some(Press::Marquee(Marquee {
                    origin: at,
                    base: view.selection.set.clone(),
                    ctrl: mods.ctrl,
                    band: None,
                }));
            }
        }
        if view.selection != before {
            self.relayout_desktop();
        }
        if let Some(i) = open {
            self.open_icon(i);
        }
    }

    /// The button came up after a press that kept a multi-selection (`Press::Icon`):
    /// without a drag that icon is selected alone; after one the set stays.
    pub fn settle_click(&mut self, dragged: bool) {
        let Some(view) = self.desktop.as_mut() else {
            return;
        };
        let pending = match view.press {
            Some(Press::Icon(i)) => {
                view.press = None;
                Some(i)
            }
            _ => None,
        };
        if let (Some(i), false) = (pending, dragged) {
            self.select_icon(Some(i));
        }
    }

    /// The pointer moved to (`lx`, `ly`) with a marquee armed: past
    /// `desktop::DRAG_PX` the band spans the press point and the pointer and every
    /// icon it touches is selected (toggled against the press-time set with Ctrl).
    pub fn drag_marquee(&mut self, lx: f64, ly: f64) {
        let Some(view) = self.desktop.as_mut() else {
            return;
        };
        let Some(Press::Marquee(m)) = view.press.as_mut() else {
            return;
        };
        let to = (lx as i32, ly as i32);
        if m.band.is_none()
            && (to.0 - m.origin.0).abs() < desktop::DRAG_PX
            && (to.1 - m.origin.1).abs() < desktop::DRAG_PX
        {
            return;
        }
        let band = selection::band(m.origin, to);
        if m.band == Some(band) {
            return;
        }
        m.band = Some(band);
        let (base, ctrl) = (m.base.clone(), m.ctrl);
        let before = view.selection.clone();
        view.selection.marquee(&view.cells, band, &base, ctrl);
        if view.selection != before {
            self.relayout_desktop();
        }
        self.draw_band(Some(band));
    }

    pub fn end_marquee(&mut self) {
        let Some(view) = self.desktop.as_mut() else {
            return;
        };
        if let Some(Press::Marquee(m)) = view.press.take()
            && m.band.is_some()
        {
            self.draw_band(None);
        }
    }

    /// Show `band`'s four dotted edges, or hide them on `None`. The dots are the
    /// desktop colour's inverse; each edge crops one of two cached strips, so a
    /// pointer step draws nothing. A look with a selection box (Glass) shows that
    /// box instead.
    fn draw_band(&mut self, band: Option<Rect>) {
        let out = self.first_output();
        let (scale, screen) = match &out {
            Some(o) => (self.draw_scale_at(Some(o)).max(1), self.layout_box(Some(o))),
            None => (1, Rect::default()),
        };
        let ink = self.config.theme.colors.background.inverted();
        let boxed = reprisekit::look::widgets::selection_box(&self.config.theme.look);
        let Some(view) = self.desktop.as_mut() else {
            return;
        };
        let nodes = view.band_nodes;
        let band = band.filter(|b| !b.is_empty());
        if let Some(rects) = view.band_box
            && (band.is_none() || boxed.is_none())
        {
            for n in rects {
                self.scene.set_enabled(n, false);
            }
        }
        let Some(b) = band else {
            for n in nodes {
                self.scene.set_enabled(n, false);
            }
            self.redraw_all();
            return;
        };
        if let Some(sel) = boxed {
            for n in nodes {
                self.scene.set_enabled(n, false);
            }
            let tree = view.tree;
            let rects = *view
                .band_box
                .get_or_insert_with(|| crate::snapview::selection_box_nodes(&mut self.scene, tree));
            crate::snapview::place_selection_box(&mut self.scene, &rects, sel, b);
            self.redraw_all();
            return;
        }
        // One logical px of slack for the phase shift in `band_crop`.
        let (need_w, need_h) = (b.width + 1, b.height + 1);
        let fits = view
            .band_strip
            .is_some_and(|(s, c, w, h)| s == scale && c == ink && w >= need_w && h >= need_h);
        if !fits {
            let (w, h) = (need_w.max(screen.width + 1), need_h.max(screen.height + 1));
            view.band_strip = Some((scale, ink, w, h));
            let across = band_strip(w, false, scale, ink);
            let down = band_strip(h, true, scale, ink);
            for (n, strip) in nodes.into_iter().zip([&across, &across, &down, &down]) {
                let (w, h) = (strip.width as i32, strip.height as i32);
                self.scene.buffer_set_canvas(n, &strip.pixels, w, h);
            }
        }
        let edges = [
            (Rect::new(b.x, b.y, b.width, 1), false),
            (Rect::new(b.x, b.bottom() - 1, b.width, 1), false),
            (Rect::new(b.x, b.y, 1, b.height), true),
            (Rect::new(b.right() - 1, b.y, 1, b.height), true),
        ];
        for (n, (edge, vertical)) in nodes.into_iter().zip(edges) {
            let crop = band_crop(edge, vertical, scale);
            let source = Rectangle::new(
                (f64::from(crop.x), f64::from(crop.y)).into(),
                (f64::from(crop.width), f64::from(crop.height)).into(),
            );
            self.scene.buffer_set_source_box(n, Some(source));
            self.scene
                .buffer_set_dest_size(n, Some((edge.width, edge.height).into()));
            self.scene.set_position(n, edge.x, edge.y);
            self.scene.raise_to_top(n);
            self.scene.set_enabled(n, true);
        }
        self.redraw_all();
    }

    pub fn drag_sources(&self, index: usize) -> Vec<crate::dragview::DragSource> {
        self.aimed_sources(index, false)
    }

    fn aimed_sources(&self, index: usize, files_only: bool) -> Vec<crate::dragview::DragSource> {
        let Some(view) = self.desktop.as_ref() else {
            return Vec::new();
        };
        aimed_in(
            &view.entries,
            &self.selection_around(index),
            index,
            files_only,
        )
        .into_iter()
        .map(|i| source_of(&view.entries[i]))
        .collect()
    }

    fn send_to(&mut self, index: usize, row: usize) {
        use reprisefs::sendto::SendTo;
        let Some(view) = self.desktop.as_ref() else {
            return;
        };
        let Some(dest) = view.send_to.get(row).map(|r| r.dest.clone()) else {
            return;
        };
        let existing: Vec<String> = view.entries.iter().map(|e| e.name.clone()).collect();
        let sources = self.aimed_sources(index, true);
        let paths: Vec<PathBuf> = sources.iter().map(|s| s.path().to_path_buf()).collect();
        match dest {
            SendTo::Zip => {
                if let Some(argv) = reprisefs::sendto::zip_argv(&paths, &existing) {
                    self.spawn_argv_activated(&argv);
                }
            }
            SendTo::DesktopShortcut => self.shortcuts_beside(index, &sources),
            SendTo::Mail => self.spawn_argv_activated(&reprisefs::sendto::mail_argv(&paths)),
            SendTo::Folder(dir) => self.perform_all(
                &sources,
                &crate::dragview::DropTarget::Folder(dir),
                crate::dragview::DropOp::Copy,
            ),
        }
    }

    pub(crate) fn select_icon(&mut self, index: Option<usize>) {
        let Some(view) = self.desktop.as_mut() else {
            return;
        };
        let index = index.filter(|&i| i < view.entries.len());
        let before = view.selection.clone();
        view.selection.set_only(index);
        if view.selection != before {
            self.relayout_desktop();
        }
    }

    /// A right press on icon `index`: one outside the selection becomes the only
    /// selected one, one inside keeps the set (its menu is for all of them).
    pub fn desktop_right_press(&mut self, index: usize) {
        let keep = self.desktop.as_ref().is_some_and(|v| v.selection.is(index));
        if !keep {
            self.select_icon(Some(index));
        }
    }

    pub(crate) fn selection_around(&self, index: usize) -> Vec<usize> {
        match self.desktop.as_ref() {
            Some(v) if v.selection.len() > 1 && v.selection.is(index) => {
                v.selection.set.iter().copied().collect()
            }
            _ => vec![index],
        }
    }

    fn aimed_files(&self, index: usize) -> Vec<PathBuf> {
        let Some(view) = self.desktop.as_ref() else {
            return Vec::new();
        };
        aimed_in(&view.entries, &self.selection_around(index), index, true)
            .into_iter()
            .map(|i| view.entries[i].path.clone())
            .filter(|p| !p.is_dir())
            .collect()
    }

    pub fn open_icon(&mut self, index: usize) {
        let Some(view) = self.desktop.as_ref() else {
            return;
        };
        let Some(entry) = view.entries.get(index) else {
            return;
        };
        let argv: Vec<std::ffi::OsString> = match &entry.target {
            Target::Launch(app) => apps::launch_argv(app, &self.config.terminal),
            Target::Open(path) => reprisefs::props::open_argv_in(
                path,
                &view.mime,
                &self.app_index,
                &self.config.terminal,
            ),
            Target::Uri(_) => match desktop::place_argv(entry) {
                Some(argv) => argv.into_iter().map(Into::into).collect(),
                None => return,
            },
        };
        self.spawn_argv_activated(&argv);
    }

    pub(crate) fn open_selection(&mut self, index: usize) {
        for i in self.selection_around(index) {
            self.open_icon(i);
        }
    }

    pub(crate) fn icon_menu_rows(
        &self,
        index: usize,
    ) -> Option<(Vec<MenuItem>, Vec<Option<DesktopCommand>>)> {
        use desktop::Kind;
        let view = self.desktop.as_ref()?;
        let entry = view.entries.get(index)?;
        Some(match entry.kind {
            Kind::RecycleBin => {
                bin_menu(reprisefs::trash::Bins::system().is_none_or(|b| b.all_empty()))
            }
            Kind::Place(place) => place_menu(place, matches!(entry.target, Target::Open(_))),
            Kind::File => {
                let is_dir = entry.path.is_dir();
                let (print, edit) = if is_dir {
                    (false, false)
                } else {
                    let mime = apps::mime_of_in(&view.mime, &entry.path);
                    reprisefs::props::verbs(mime.as_deref(), reprisefs::props::has_lp())
                };
                icon_menu(IconMenu {
                    is_dir,
                    print,
                    edit,
                    allow_launching: !is_dir && desktop::can_allow_launching(entry),
                    launcher: matches!(entry.target, Target::Launch(_)),
                    in_set: self.selection_around(index).len() > 1,
                })
            }
        })
    }

    pub(crate) fn open_with_apps(&self, target: &IconId) -> Vec<(String, String)> {
        let Some(view) = self.desktop.as_ref() else {
            return Vec::new();
        };
        let path = desktop::find(&view.entries, target).map(|i| &view.entries[i].path);
        let index = &self.app_index;
        path.and_then(|p| apps::mime_of_in(&view.mime, p))
            .map(|mime| apps::open_with(index, &mime))
            .unwrap_or_default()
            .into_iter()
            .map(|i| (index[i].id.clone(), index[i].name.clone()))
            .collect()
    }

    pub(crate) fn desktop_auto_arrange(&self) -> bool {
        self.desktop.as_ref().is_some_and(|d| d.auto_arrange)
    }

    pub(crate) fn place_icon(&mut self, name: &str, cell: (i32, i32)) {
        self.pin_icon(name, cell);
        self.relayout_desktop();
    }

    /// Remember grid cell `cell` for the icon called `name` and save it, without
    /// redrawing.
    pub(crate) fn pin_icon(&mut self, name: &str, cell: (i32, i32)) {
        let Some(view) = self.desktop.as_mut() else {
            return;
        };
        // One icon per cell: whoever held it before falls back into the flow.
        view.positions.retain(|_, c| *c != cell);
        view.positions.insert(name.to_string(), cell);
        crate::state::save_desktop(view.auto_arrange, &view.positions);
    }

    fn arrange_by_name(&mut self) {
        if let Some(view) = self.desktop.as_mut() {
            view.positions.clear();
            crate::state::save_desktop(view.auto_arrange, &view.positions);
        }
        self.relayout_desktop();
    }

    /// Toggle Auto Arrange. Turning it on also drops the positions.
    fn toggle_auto_arrange(&mut self) {
        if let Some(view) = self.desktop.as_mut() {
            view.auto_arrange = !view.auto_arrange;
            if view.auto_arrange {
                view.positions.clear();
            }
            crate::state::save_desktop(view.auto_arrange, &view.positions);
        }
        self.relayout_desktop();
    }

    /// A new item `name` exists on disk: pin it to the free cell nearest `at` (unless
    /// Auto Arrange flows it), then rescan once.
    fn created(&mut self, name: &str, at: Option<(i32, i32)>) {
        let area = self
            .first_output()
            .map(|o| self.usable_area(Some(&o)))
            .filter(|a| a.width > 0);
        let cell = self.desktop.as_ref().and_then(|view| {
            let ((x, y), area) = (at?, area?);
            if view.auto_arrange {
                return None;
            }
            let square = desktop::Cell::square(desktop::CELL);
            let taken: std::collections::HashSet<(i32, i32)> = view
                .cells
                .iter()
                .map(|r| desktop::cell_at(square, area, r.x, r.y))
                .collect();
            Some(desktop::free_cell_near(
                area,
                &taken,
                desktop::cell_at(square, area, x, y),
            ))
        });
        if let Some(cell) = cell {
            self.pin_icon(name, cell);
        }
        self.rescan_desktop();
    }

    /// Create a new folder with a unique name ("New Folder", "New Folder (2)", ...)
    /// in the cell under `at` or the next free one. Does nothing while the desktop's
    /// files are off.
    fn new_folder(&mut self, at: Option<(i32, i32)>) {
        if !self.config.desktop_icons {
            return;
        }
        let Some(view) = self.desktop.as_ref() else {
            return;
        };
        let existing: Vec<String> = view.entries.iter().map(|e| e.name.clone()).collect();
        let name = names::new_folder_name(&existing);
        let path = view.dir.join(&name);
        match std::fs::create_dir(&path) {
            Ok(()) => self.created(&name, at),
            Err(e) => log::warn!("desktop: could not create {}: {e}", path.display()),
        }
    }

    /// Create an empty text file at the first free "New Text Document.txt", "New Text
    /// Document (2).txt", ..., claimed with `O_EXCL`. Does nothing while the
    /// desktop's files are off.
    fn new_text_document(&mut self, at: Option<(i32, i32)>) {
        if !self.config.desktop_icons {
            return;
        }
        let Some(view) = self.desktop.as_ref() else {
            return;
        };
        match ops::create_new_numbered(&view.dir, "New Text Document.txt") {
            Ok((_, path)) => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                self.created(&name, at)
            }
            Err(e) => log::warn!("desktop: could not create a text document: {e}"),
        }
    }

    /// Run a desktop-menu command; `aim` is the icon it was aimed at, which may have
    /// since gone.
    pub fn desktop_command(&mut self, cmd: DesktopCommand, aim: Option<IconId>) {
        use DesktopCommand as Dc;
        let target = match &aim {
            Some(id) => {
                let found = self
                    .desktop
                    .as_ref()
                    .and_then(|v| desktop::find(&v.entries, id));
                if found.is_none() {
                    return;
                }
                found
            }
            None => None,
        };
        match cmd {
            Dc::Refresh => self.rescan_desktop(),
            Dc::NextBackground => self.next_slide(),
            Dc::NewFolder => self.new_folder(self.desktop_menu_at),
            Dc::NewTextDocument => self.new_text_document(self.desktop_menu_at),
            Dc::Open => {
                if let Some(i) = target {
                    self.open_selection(i);
                }
            }
            Dc::Explore => {
                if let Some(i) = target {
                    self.open_icon(i);
                }
            }
            Dc::Trash => {
                if let Some(i) = target {
                    self.trash_selection(i);
                }
            }
            Dc::Rename => {
                if let Some(i) = target {
                    self.begin_rename(i);
                }
            }
            Dc::CreateShortcut => {
                if let Some(i) = target {
                    let sources = self.drag_sources(i);
                    self.shortcuts_beside(i, &sources);
                }
            }
            Dc::Cut | Dc::Copy => {
                if let Some(i) = target {
                    self.copy_icons(i, cmd == Dc::Cut);
                }
            }
            Dc::Paste | Dc::PasteShortcut => {
                // The same as dropping the files on the cell under the
                // right click.
                let area = self.desktop_area();
                let at = self.desktop_menu_at.unwrap_or((area.x, area.y));
                let cell = desktop::cell_at(desktop::Cell::square(desktop::CELL), area, at.0, at.1);
                let target = crate::dragview::DropTarget::Desktop(cell);
                self.paste_files(target, cmd == Dc::PasteShortcut);
            }
            Dc::PasteInto => {
                let dir = target
                    .and_then(|i| self.desktop.as_ref()?.entries.get(i))
                    .map(|e| e.path.clone());
                if let Some(dir) = dir {
                    self.paste_files(crate::dragview::DropTarget::Folder(dir), false);
                }
            }
            Dc::AllowLaunching => {
                let Some(i) = target else {
                    return;
                };
                // The row shows for a launcher that is not trusted yet;
                // the rest of the set gets the same where it is one.
                let set = self.selection_around(i);
                let paths: Vec<PathBuf> = match self.desktop.as_ref() {
                    Some(view) => aimed_in(&view.entries, &set, i, true)
                        .into_iter()
                        .map(|i| &view.entries[i])
                        .filter(|e| desktop::can_allow_launching(e))
                        .map(|e| e.path.clone())
                        .collect(),
                    None => Vec::new(),
                };
                let failed = paths
                    .iter()
                    .find_map(|p| apps::allow_launching(p).err().map(|e| (p, e)));
                if let Some((path, e)) = failed {
                    let name = path
                        .file_name()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy();
                    let text = format!("Cannot allow {name} to launch: {e}");
                    self.open_dialog(crate::dialog::Dialog::error("Allow Launching", text));
                }
                self.rescan_desktop();
            }
            Dc::Settings => {
                self.spawn_argv_activated(&["reprise-settings", "--page", "background"])
            }
            Dc::RepriseSettings => self.spawn_argv_activated(&["reprise-settings"]),
            Dc::AboutReprise => self.spawn_argv_activated(&["reprise-about"]),
            Dc::OpenWithApp(app) => {
                let Some(i) = target else {
                    return;
                };
                let Some(app) = self.app_index.iter().find(|a| a.id == app).cloned() else {
                    return;
                };
                for path in self.aimed_files(i) {
                    let argv = apps::open_argv(&app, &self.config.terminal, &path);
                    self.spawn_argv_activated(&argv);
                }
            }
            Dc::Print | Dc::Edit => {
                let Some(i) = target else {
                    return;
                };
                // Edit: the text/plain default, whatever the file's own type.
                let editor =
                    reprisefs::props::default_app_entry("text/plain", &self.app_index).cloned();
                for path in self.aimed_files(i) {
                    let argv = if cmd == Dc::Print {
                        vec!["lp".into(), path.into_os_string()]
                    } else {
                        reprisefs::props::open_argv(&path, editor.as_ref(), &self.config.terminal)
                    };
                    self.spawn_argv_activated(&argv);
                }
            }
            Dc::ArrangeByName => self.arrange_by_name(),
            Dc::AutoArrange => self.toggle_auto_arrange(),
            Dc::DropCopy | Dc::DropMove | Dc::DropShortcut => {
                use crate::dragview::DropOp;
                let op = match cmd {
                    Dc::DropCopy => DropOp::Copy,
                    Dc::DropMove => DropOp::Move,
                    _ => DropOp::Shortcut,
                };
                // Another program's drop hears the answer first, and one
                // its source does not allow is cancelled.
                if let Some((sources, target)) = self.drags.pending_drop.take()
                    && self.drop_menu_answered(Some(op))
                {
                    self.perform_all(&sources, &target, op);
                }
            }
            Dc::DropCancel => {
                self.drags.pending_drop = None;
                self.drop_menu_answered(None);
            }
            Dc::SendToDest(row) => {
                if let Some(i) = target {
                    self.send_to(i, row);
                }
            }
            // Only ever a submenu opener; picking it does nothing.
            Dc::ArrangeIcons | Dc::New | Dc::OpenWith | Dc::SendTo => {}
            Dc::IconProperties => {
                if let Some(i) = target {
                    self.open_icon_properties(i);
                }
            }
            Dc::ChooseProgram => {
                if let Some(i) = target {
                    self.open_icon_chooser(i);
                }
            }
            Dc::BinProperties => {
                if let Some(i) = target {
                    self.open_bin_properties(i);
                }
            }
            Dc::EmptyBin => self.ask_empty_bin(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Draws the desktop menu, the place menus and the folder and file icon menus
    /// under Classic into `target/`, for `scripts/pixels-unchanged.sh`.
    #[test]
    #[ignore]
    fn render_desktop_menu() {
        use reprisekit::controls::menu::{MenuLayout, MenuStyle, draw_menu};
        use reprisekit::raster::{Canvas, write_preview};
        use reprisekit::text::TextRenderer;
        use reprisekit::theme::{FontConfig, Theme};
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let draw = |items: &[MenuItem], underline: bool| {
            let style = MenuStyle::context_for(&theme, items).fit(
                &text,
                items,
                theme.fonts.size_px,
                i32::MAX,
            );
            let layout = MenuLayout::compute_styled(&style, items, usize::MAX);
            let mut c = Canvas::new(layout.width as u32, layout.height as u32);
            draw_menu(&mut c, &theme, &text, &layout, items, None, "", underline);
            c
        };
        write_preview(
            &draw(&desktop_menu(false, true, false).0, false),
            "desktopmenu-preview.png",
        );
        let places = [
            place_menu(desktop::Place::Documents, true).0,
            place_menu(desktop::Place::Computer, false).0,
            place_menu(desktop::Place::Home, true).0,
        ];
        let canvases: Vec<Canvas> = places.iter().map(|items| draw(items, true)).collect();
        let w: u32 = canvases.iter().map(|c| c.width + 8).sum();
        let h = canvases.iter().map(|c| c.height).max().unwrap_or(1);
        let mut all = Canvas::new(w, h);
        let mut x = 0;
        for c in &canvases {
            all.blit_canvas(x, 0, c);
            x += c.width as i32 + 8;
        }
        write_preview(&all, "placemenus-preview.png");
        for (name, items) in [
            (
                "folder-icon-menu",
                icon_menu(IconMenu {
                    is_dir: true,
                    ..Default::default()
                })
                .0,
            ),
            (
                "file-icon-menu",
                icon_menu(IconMenu {
                    print: true,
                    edit: true,
                    ..Default::default()
                })
                .0,
            ),
        ] {
            write_preview(&draw(&items, false), &format!("{name}-preview.png"));
        }
    }

    #[test]
    fn send_to_menu_greys_a_disabled_row() {
        let rows = reprisefs::sendto::rows(
            None,
            Some(PathBuf::from("/home/u/Documents")),
            &[PathBuf::from("/run/media/u/STICK")],
        );
        let (items, commands) = send_to_menu(&rows);
        assert_eq!(items.len(), 5);
        assert_eq!(commands[1], None, "no desktop folder, no Desktop row");
        assert_eq!(commands[3], Some(DesktopCommand::SendToDest(3)));
    }

    fn file(name: &str) -> DesktopEntry {
        let path = PathBuf::from("/d").join(name);
        DesktopEntry {
            name: name.into(),
            label: name.into(),
            icon: "text-x-generic".into(),
            target: Target::Open(path.clone()),
            path,
            shortcut: false,
            kind: desktop::Kind::File,
        }
    }

    /// `desktop_icons` off hides only the folder's files: the system icons
    /// still list, and the folder is neither read nor watched.
    #[test]
    fn the_system_icons_list_with_the_desktop_files_off() {
        let system_icons: Vec<String> = ["documents", "recycle-bin"].map(String::from).into();
        let system = |name: &str| match name {
            "recycle-bin" => Some(desktop::recycle_bin(PathBuf::from("/t/files"), true)),
            _ => desktop::place(name),
        };
        let names = |entries: &[DesktopEntry]| -> Vec<String> {
            entries.iter().map(|e| e.label.clone()).collect()
        };
        let off = listed(&system_icons, false, system, || {
            panic!("the folder is not read with its files off")
        });
        assert_eq!(names(&off), ["Documents", "Recycle Bin"]);
        let on = listed(&system_icons, true, system, || vec![file("a.txt")]);
        assert_eq!(names(&on), ["Documents", "Recycle Bin", "a.txt"]);

        let dir = Path::new("/home/u/Desktop");
        let watched = |files: bool| -> Vec<PathBuf> {
            watch_wants(dir, files, &[])
                .into_iter()
                .map(|w| w.dir)
                .collect()
        };
        assert_eq!(watched(false), Vec::<PathBuf>::new(), "nothing to watch");
        assert_eq!(
            watched(true),
            [dir.to_path_buf(), PathBuf::from("/home/u")],
            "the folder and its parent"
        );
    }

    #[test]
    fn a_set_command_takes_the_clicked_icon_first_then_the_rest() {
        let docs = desktop::place("documents").unwrap();
        let bin = desktop::recycle_bin(PathBuf::from("/t/files"), true);
        let entries = vec![docs, file("a.txt"), file("b.txt"), bin];
        assert_eq!(
            aimed_in(&entries, &[0, 1, 2, 3], 2, false),
            vec![2, 0, 1, 3]
        );
        // Cut, Copy and Send To leave the places and the bin out of the rest.
        assert_eq!(aimed_in(&entries, &[0, 1, 2, 3], 2, true), vec![2, 1]);
        // A place clicked in the set still takes itself.
        assert_eq!(aimed_in(&entries, &[0, 1], 0, true), vec![0, 1]);
        // An index past the entries (a rescan shrank them) is dropped.
        assert_eq!(aimed_in(&entries, &[1, 9], 1, false), vec![1]);
    }

    #[test]
    fn cell_key_owned_copy_matches_its_borrowed_key_until_the_caret_blinks() {
        let (label, text) = (String::from("Notes"), String::from("Notes"));
        let key = |on| CellKey {
            label: Cow::Borrowed(&label),
            icon: Cow::Borrowed("text-x-generic"),
            shortcut: false,
            cell: Rect::default(),
            selected: true,
            ring: false,
            cut: false,
            editing: Some((Cow::Borrowed(text.as_str()), 0..5, 5, on)),
            preedit: None,
        };
        let stored: CellKey<'static> = key(true).into_owned();
        assert!(matches!(stored.label, Cow::Owned(_)));
        assert!(stored == key(true));
        assert!(stored != key(false));
        let composing = CellKey {
            preedit: reprisekit::textedit::Preedit::new("ni".into(), 2, 2),
            ..key(true)
        };
        assert!(stored != composing, "a preedit redraws the box");
    }

    #[test]
    fn remap_follows_the_file_and_drops_a_gone_entry() {
        let entry = |name: &str| DesktopEntry {
            name: name.into(),
            label: name.into(),
            icon: "text-x-generic".into(),
            target: Target::Open(PathBuf::from(name)),
            path: PathBuf::from(name),
            shortcut: false,
            kind: desktop::Kind::File,
        };
        let bin = DesktopEntry {
            kind: desktop::Kind::RecycleBin,
            ..entry("Recycle Bin")
        };
        let old = [entry("b.txt"), entry("c.txt"), bin.clone()];
        let new = [bin, entry("a.txt"), entry("b.txt")];
        assert_eq!(
            remap(&old, &new),
            [Some(2), None, Some(0)],
            "b.txt moves to row 2, c.txt is gone, the bin moves to row 0"
        );
    }

    #[test]
    fn a_launchers_name_key_moves_to_its_file_name_once() {
        let launcher = |file: &str, name: &str| DesktopEntry {
            name: file.into(),
            label: name.into(),
            icon: "firefox".into(),
            target: Target::Launch(
                apps::parse_desktop(
                    &format!("[Desktop Entry]\nType=Application\nName={name}\nExec=x\n"),
                    file,
                )
                .unwrap(),
            ),
            path: PathBuf::from(file),
            shortcut: true,
            kind: desktop::Kind::File,
        };
        let plain = |name: &str| DesktopEntry {
            name: name.into(),
            label: name.into(),
            icon: "text-x-generic".into(),
            target: Target::Open(PathBuf::from(name)),
            path: PathBuf::from(name),
            shortcut: false,
            kind: desktop::Kind::File,
        };
        let entries = [
            launcher("firefox.desktop", "Firefox"),
            launcher("firefox (2).desktop", "Firefox"),
            launcher("notes.desktop", "notes.txt"),
            plain("notes.txt"),
        ];
        let mut positions = BTreeMap::from([
            ("Firefox".to_string(), (3, 1)),
            ("notes.txt".to_string(), (0, 5)),
        ]);
        assert!(migrate_launcher_keys(&entries, &mut positions));
        assert_eq!(
            positions,
            BTreeMap::from([
                ("firefox.desktop".to_string(), (3, 1)),
                ("notes.txt".to_string(), (0, 5)),
            ]),
            "the first Firefox takes the cell; notes.txt keeps its own"
        );
        assert!(!migrate_launcher_keys(&entries, &mut positions));
    }

    #[test]
    fn recycle_bin_menu_greys_empty_and_entry_cannot_be_trashed() {
        use DesktopCommand::*;
        let (items, commands) = bin_menu(true);
        assert_eq!(
            commands,
            vec![
                Some(Open),
                Some(Explore),
                None,
                None,
                Some(CreateShortcut),
                None,
                Some(BinProperties)
            ]
        );
        assert!(
            matches!(&items[2], MenuItem::Entry { enabled: false, label, .. } if label == "Empty Recycle Bin")
        );
        assert!(matches!(items[0], MenuItem::Entry { bold: true, .. }));
        let (_, commands) = bin_menu(false);
        assert_eq!(commands[2], Some(EmptyBin));
        // The place menus: Documents and Home have Explore and Copy, and
        // Properties wherever there is a folder behind them; Computer has
        // Reprise Settings and About Reprise last, after a separator. All
        // three Delete and Rename.
        let (items, commands) = place_menu(desktop::Place::Documents, true);
        let folder_rows = vec![
            Some(Open),
            Some(Explore),
            None,
            Some(Copy),
            None,
            Some(CreateShortcut),
            Some(Trash),
            Some(Rename),
            None,
            Some(IconProperties),
        ];
        assert_eq!(commands, folder_rows);
        assert_eq!(place_menu(desktop::Place::Home, true), (items, commands));
        let (items, commands) = place_menu(desktop::Place::Computer, false);
        assert_eq!(
            commands,
            vec![
                Some(Open),
                None,
                Some(CreateShortcut),
                Some(Trash),
                Some(Rename),
                None,
                Some(RepriseSettings),
                Some(AboutReprise),
            ]
        );
        let rows: Vec<(String, Option<usize>, bool)> = items
            .iter()
            .filter_map(|i| match i {
                MenuItem::Entry {
                    label,
                    mnemonic,
                    bold,
                    ..
                } => Some((label.clone(), *mnemonic, *bold)),
                _ => None,
            })
            .collect();
        assert_eq!(
            rows,
            [
                ("Open".to_string(), Some(0), true),
                ("Create Shortcut".to_string(), Some(7), false),
                ("Delete".to_string(), Some(0), false),
                ("Rename".to_string(), Some(4), false),
                ("Reprise Settings".to_string(), Some(0), false),
                ("About Reprise".to_string(), Some(0), false),
            ]
        );
        let keys: std::collections::HashSet<char> = rows
            .iter()
            .map(|(l, m, _)| l.chars().nth(m.unwrap()).unwrap().to_ascii_lowercase())
            .collect();
        assert_eq!(keys.len(), rows.len(), "no two rows share an access key");
        let pc = desktop::place("computer").unwrap();
        assert_eq!(
            (pc.name.as_str(), pc.icon.as_str()),
            ("Computer", "computer")
        );
        assert!(matches!(&pc.target, Target::Uri(u) if u == "computer:///"));
        assert_eq!(pc.kind, desktop::Kind::Place(desktop::Place::Computer));
        assert!(desktop::place("my-music").is_none());
        let cmd = desktop::place_command(&pc);
        assert_eq!(cmd.as_deref(), Some("reprise-explorer Computer"));
        let bin = desktop::recycle_bin(PathBuf::from("/t/files"), false);
        assert_eq!(
            desktop::place_command(&bin).as_deref(),
            Some("reprise-explorer \"Recycle Bin\"")
        );
        assert_eq!(
            (bin.name.as_str(), bin.icon.as_str()),
            ("Recycle Bin", "user-trash-full")
        );
        assert_eq!(
            desktop::recycle_bin(PathBuf::from("/t/files"), true).icon,
            "user-trash"
        );
        assert_eq!(bin.kind, desktop::Kind::RecycleBin);
    }

    #[test]
    fn a_slideshow_adds_next_desktop_background_after_refresh() {
        let (items, commands) = desktop_menu(false, true, true);
        assert_eq!(items.len(), 11);
        assert_eq!(commands[1], Some(DesktopCommand::Refresh));
        assert_eq!(commands[2], Some(DesktopCommand::NextBackground));
        assert!(matches!(
            &items[2],
            MenuItem::Entry { label, mnemonic: Some(13), enabled: true, .. }
                if label == "Next desktop background"
        ));
        assert!(matches!(items[3], MenuItem::Separator));
        assert_eq!(desktop_menu(false, true, false).0.len(), 10);
    }

    #[test]
    fn with_the_files_off_the_desktop_menu_greys_new_and_paste() {
        let (items, commands) = desktop_menu(true, false, false);
        for i in [3, 4, 6] {
            assert!(
                matches!(items[i], MenuItem::Entry { enabled: false, .. }),
                "row {i} is greyed with the desktop's files off"
            );
            assert_eq!(commands[i], None, "row {i} does nothing");
        }
        // New keeps its submenu arrow, greyed.
        assert!(matches!(items[6], MenuItem::Entry { submenu: true, .. }));
        for i in [0, 1, 8, 9] {
            assert!(matches!(items[i], MenuItem::Entry { enabled: true, .. }));
        }
        assert_eq!(commands[1], Some(DesktopCommand::Refresh));
        assert_eq!(commands[8], Some(DesktopCommand::Settings));
        assert_eq!(commands[9], Some(DesktopCommand::AboutReprise));
    }

    #[test]
    fn desktop_menu_lists_its_commands_with_separators() {
        let (items, commands) = desktop_menu(false, true, false);
        assert_eq!(items.len(), 10);
        assert_eq!(commands[0], Some(DesktopCommand::ArrangeIcons));
        assert!(matches!(items[0], MenuItem::Entry { submenu: true, .. }));
        assert_eq!(commands[1], Some(DesktopCommand::Refresh));
        assert!(matches!(items[2], MenuItem::Separator));
        // Paste and Paste Shortcut are there, greyed out with nothing
        // copied.
        assert!(matches!(items[3], MenuItem::Entry { enabled: false, .. }));
        assert!(matches!(items[4], MenuItem::Entry { enabled: false, .. }));
        assert_eq!(commands[3], None);
        assert!(matches!(items[5], MenuItem::Separator));
        assert_eq!(commands[6], Some(DesktopCommand::New));
        assert!(matches!(items[6], MenuItem::Entry { submenu: true, .. }));
        assert!(matches!(items[7], MenuItem::Separator));
        assert_eq!(commands[8], Some(DesktopCommand::Settings));
        assert!(matches!(
            &items[8],
            MenuItem::Entry { label, mnemonic: Some(0), .. } if label == "Reprise Settings"
        ));
        assert_eq!(commands[9], Some(DesktopCommand::AboutReprise));
        assert!(matches!(
            &items[9],
            MenuItem::Entry { label, mnemonic: Some(0), .. } if label == "About Reprise"
        ));
        let (items, commands) = new_menu();
        assert_eq!(items.len(), NEW_MENU_ICONS.len());
        assert_eq!(commands[0], Some(DesktopCommand::NewFolder));
        assert!(matches!(items[1], MenuItem::Separator));
        assert_eq!(commands[2], Some(DesktopCommand::NewTextDocument));
        let (items, commands) = arrange_menu(true);
        assert_eq!(
            commands,
            vec![
                Some(DesktopCommand::ArrangeByName),
                None,
                Some(DesktopCommand::AutoArrange)
            ]
        );
        assert!(matches!(items[2], MenuItem::Entry { checked: true, .. }));
        assert!(matches!(
            arrange_menu(false).0[2],
            MenuItem::Entry { checked: false, .. }
        ));
        // With something on the clipboard the Paste pair comes alive.
        let (items, commands) = desktop_menu(true, true, false);
        assert!(matches!(items[3], MenuItem::Entry { enabled: true, .. }));
        assert_eq!(commands[3], Some(DesktopCommand::Paste));
        assert_eq!(commands[4], Some(DesktopCommand::PasteShortcut));
        let (items, _) = desktop_menu(false, true, false);
        for i in [0, 1, 3, 4, 6, 8, 9] {
            assert!(matches!(
                items[i],
                MenuItem::Entry {
                    mnemonic: Some(_),
                    ..
                }
            ));
        }
    }

    #[test]
    fn icon_menu_has_its_rows_with_open_bold() {
        use DesktopCommand::*;
        let (items, commands) = icon_menu(IconMenu::default());
        assert_eq!(
            commands,
            vec![
                Some(Open),
                Some(OpenWith),
                None,
                Some(SendTo),
                None,
                Some(Cut),
                Some(Copy),
                None,
                Some(CreateShortcut),
                Some(Trash),
                Some(Rename),
                None,
                Some(IconProperties)
            ]
        );
        assert!(matches!(items[0], MenuItem::Entry { bold: true, .. }));
        assert!(matches!(items[2], MenuItem::Separator));
        assert!(matches!(items[3], MenuItem::Entry { submenu: true, .. }));
        assert!(matches!(items[7], MenuItem::Separator));
        // A folder gets Explore and the greyed Sharing and Security...
        // under Open.
        let (items, commands) = icon_menu(IconMenu {
            is_dir: true,
            print: true,
            edit: true,
            ..Default::default()
        });
        assert_eq!(commands[1], Some(Explore));
        assert!(matches!(&items[1], MenuItem::Entry { label, .. } if label == "Explore"));
        assert!(matches!(&items[2], MenuItem::Separator));
        assert!(matches!(
            &items[3],
            MenuItem::Entry { label, enabled: false, .. } if label == "Sharing and Security..."
        ));
        assert_eq!(commands[3], None);
        assert_eq!(commands[5], Some(SendTo));
        assert_eq!(commands[7], Some(Cut));
        // A file gets Open With, a submenu, there instead, with its verbs
        // before it: Print, then Edit.
        let (items, commands) = icon_menu(IconMenu::default());
        assert_eq!(commands[1], Some(OpenWith));
        assert!(matches!(&items[1], MenuItem::Entry { submenu: true, .. }));
        let (_, commands) = icon_menu(IconMenu {
            print: true,
            edit: true,
            ..Default::default()
        });
        assert_eq!(
            &commands[..4],
            &[Some(Open), Some(Print), Some(Edit), Some(OpenWith)]
        );
        let (_, commands) = icon_menu(IconMenu {
            edit: true,
            ..Default::default()
        });
        assert_eq!(&commands[..3], &[Some(Open), Some(Edit), Some(OpenWith)]);
        // A .desktop file that is not a launcher gets Allow Launching
        // under Open With.
        let (items, commands) = icon_menu(IconMenu {
            edit: true,
            allow_launching: true,
            ..Default::default()
        });
        assert_eq!(commands[3], Some(AllowLaunching));
        assert!(matches!(
            &items[3],
            MenuItem::Entry { label, mnemonic: Some(6), .. } if label == "Allow Launching"
        ));
        let (items, commands) = open_with_menu(&[
            ("vim".into(), "Vim".into()),
            ("tj".into(), "Tom & Jerry".into()),
        ]);
        assert_eq!(
            commands,
            vec![
                Some(OpenWithApp("vim".into())),
                Some(OpenWithApp("tj".into())),
                None,
                Some(ChooseProgram)
            ]
        );
        assert!(
            matches!(&items[1], MenuItem::Entry { label, mnemonic: None, .. } if label == "Tom & Jerry")
        );
        assert!(matches!(items[2], MenuItem::Separator));
        let (items, commands) = open_with_menu(&[]);
        assert_eq!(commands, vec![Some(ChooseProgram)]);
        assert!(matches!(&items[0], MenuItem::Entry { label, .. } if label == "Choose Program..."));
    }

    #[test]
    fn a_folder_offers_paste_and_a_launcher_a_greyed_run_as() {
        use DesktopCommand::*;
        let label = |items: &[MenuItem], i: usize| match &items[i] {
            MenuItem::Entry { label, enabled, .. } => (label.clone(), *enabled),
            _ => (String::new(), false),
        };
        // A folder's Paste sits under Copy, alone or in a set.
        for in_set in [false, true] {
            let (items, commands) = icon_menu(IconMenu {
                is_dir: true,
                in_set,
                ..Default::default()
            });
            let copy = commands.iter().position(|c| *c == Some(Copy)).unwrap();
            assert_eq!(label(&items, copy + 1), ("Paste".into(), true));
            assert_eq!(commands[copy + 1], Some(PasteInto));
            assert!(matches!(
                &items[copy + 1],
                MenuItem::Entry {
                    mnemonic: Some(0),
                    ..
                }
            ));
            assert!(matches!(items[copy + 2], MenuItem::Separator));
        }
        // A file's menu has no Paste.
        let (_, commands) = icon_menu(IconMenu::default());
        assert!(!commands.contains(&Some(PasteInto)));
        // A launcher's Run as... is greyed under Open; a document has none.
        let (items, commands) = icon_menu(IconMenu {
            launcher: true,
            ..Default::default()
        });
        assert_eq!(label(&items, 1), ("Run as...".into(), false));
        assert!(matches!(
            &items[1],
            MenuItem::Entry {
                mnemonic: Some(4),
                ..
            }
        ));
        assert_eq!(&commands[..3], &[Some(Open), None, Some(OpenWith)]);
        let (items, _) = icon_menu(IconMenu::default());
        assert!(
            items
                .iter()
                .all(|i| !matches!(i, MenuItem::Entry { label, .. } if label == "Run as..."))
        );
    }

    #[test]
    fn an_icon_in_a_set_keeps_its_own_menu_less_sharing() {
        use DesktopCommand::*;
        // A folder's drops the greyed Sharing and Security... and the
        // separator over it.
        let (items, commands) = icon_menu(IconMenu {
            is_dir: true,
            in_set: true,
            ..Default::default()
        });
        assert_eq!(
            commands,
            vec![
                Some(Open),
                Some(Explore),
                None,
                Some(SendTo),
                None,
                Some(Cut),
                Some(Copy),
                Some(PasteInto),
                None,
                Some(CreateShortcut),
                Some(Trash),
                Some(Rename),
                None,
                Some(IconProperties)
            ]
        );
        assert!(
            items.iter().all(
                |i| !matches!(i, MenuItem::Entry { label, .. } if label.starts_with("Sharing"))
            )
        );
        // A file's keeps its verbs and Open With.
        let set = icon_menu(IconMenu {
            print: true,
            allow_launching: true,
            in_set: true,
            ..Default::default()
        })
        .1;
        assert_eq!(
            set,
            icon_menu(IconMenu {
                print: true,
                allow_launching: true,
                ..Default::default()
            })
            .1
        );
        assert_eq!(
            &set[..4],
            &[
                Some(Open),
                Some(Print),
                Some(OpenWith),
                Some(AllowLaunching)
            ]
        );
    }

    #[test]
    fn a_band_edge_cropped_from_its_strip_matches_one_drawn_in_place() {
        use reprisekit::classic::widgets::draw_focus_ring_screen;
        let ink = reprisekit::theme::Color::rgb(255, 255, 255);
        for scale in [1, 2] {
            for (edge, vertical) in [
                (Rect::new(10, 20, 7, 1), false),
                (Rect::new(11, 20, 7, 1), false),
                (Rect::new(-3, 4, 1, 9), true),
                (Rect::new(5, 8, 1, 6), true),
            ] {
                let mut direct = crate::raster::Canvas::new(
                    (edge.width * scale) as u32,
                    (edge.height * scale) as u32,
                );
                let full = Rect::new(0, 0, edge.width * scale, edge.height * scale);
                draw_focus_ring_screen(&mut direct, full, scale, ink, (edge.x, edge.y));
                let strip = band_strip(20, vertical, scale, ink);
                let crop = band_crop(edge, vertical, scale);
                assert_eq!((crop.width, crop.height), (full.width, full.height));
                for y in 0..full.height {
                    for x in 0..full.width {
                        assert_eq!(
                            strip.get(crop.x + x, crop.y + y),
                            direct.get(x, y),
                            "edge {edge:?} at scale {scale}: px ({x}, {y}) matches the in-place ring"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_drop_menu_bolds_the_natural_row_and_greys_what_the_drag_disallows() {
        use crate::dragview::{Allowed, DropOp};
        let bold = |items: &[MenuItem]| -> Vec<bool> {
            items
                .iter()
                .map(|i| matches!(i, MenuItem::Entry { bold: true, .. }))
                .collect()
        };
        let (drop, drop_cmds) = drop_menu(false, DropOp::Move, Allowed::ALL);
        assert_eq!(drop_cmds[1], Some(DesktopCommand::DropMove));
        assert_eq!(bold(&drop), [false, true, false, false, false]);
        assert_eq!(drop_cmds[4], Some(DesktopCommand::DropCancel));
        // A drag that only allows a copy greys Move Here; one that only
        // allows a move greys Copy Here and Create Shortcuts Here.
        let enabled = |allowed| -> Vec<bool> {
            drop_menu(false, DropOp::Copy, allowed)
                .0
                .iter()
                .map(|i| matches!(i, MenuItem::Entry { enabled: true, .. }))
                .collect()
        };
        let only = |copy, moves| Allowed { copy, moves };
        assert_eq!(enabled(only(true, false)), [true, false, true, false, true]);
        assert_eq!(
            enabled(only(false, true)),
            [false, true, false, false, true]
        );
        // The row a drop would have taken is the bold one.
        assert_eq!(
            bold(&drop_menu(false, DropOp::Copy, Allowed::ALL).0),
            [true, false, false, false, false]
        );
        assert_eq!(
            bold(&drop_menu(false, DropOp::Shortcut, Allowed::ALL).0),
            [false, false, true, false, false]
        );
        // A system icon only moves or becomes a shortcut.
        let (drop, drop_cmds) = drop_menu(true, DropOp::Copy, Allowed::ALL);
        assert_eq!(
            drop_cmds,
            [
                Some(DesktopCommand::DropMove),
                Some(DesktopCommand::DropShortcut),
                None,
                Some(DesktopCommand::DropCancel),
            ]
        );
        assert!(matches!(drop[0], MenuItem::Entry { bold: true, .. }));
    }

    #[test]
    fn a_drag_carries_the_grabbed_icon_first_then_its_set() {
        let entries = [file("a.txt"), file("b.txt"), file("c.txt")];
        let sources: Vec<String> = aimed_in(&entries, &[0, 1, 2], 1, false)
            .into_iter()
            .map(|i| match source_of(&entries[i]) {
                crate::dragview::DragSource::Icon { name, .. } => name,
                other => panic!("an icon drags as an icon, not {other:?}"),
            })
            .collect();
        assert_eq!(sources, ["b.txt", "a.txt", "c.txt"]);
    }
}
