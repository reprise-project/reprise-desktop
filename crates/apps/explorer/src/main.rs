//! Explorer: the file manager. The window is glue: the chrome is
//! reprisekit's controls, the file system is reprisefs, and every input
//! resolves to a [`commands::Command`] before anything changes.

mod browse;
mod chrome;
mod commands;
mod dialogs;
mod drag;
mod fileops;
mod foldertree;
mod icons;
mod keys;
mod live;
mod menus;
mod openwith;
mod pane;
mod place;
mod pointer;
mod props_dialog;
mod props_host;
mod rename;
mod settings;
mod tree;
mod volumes;
mod watch;
mod window;

use dialogs::{DIALOG, Dialog, PROGRAM_BROWSE};
use icons::chrome_icon;
use menus::MenuState;
use place::{Listed, Place, list, sort_order_grouped};
use props_dialog::{PROPS, Props};
use repriseapp::decode::Decoders;
use repriseapp::keys::AltArm;
use repriseapp::thumbs::{self, Thumbs};
use repriseapp::{
    App, BTN_LEFT, BTN_RIGHT, Child, Ctx, DropForm, Event, FileDrop, MAIN, PointerInput, Reply,
    WinId,
};
use reprisefs::history::History;
use reprisefs::listing::Item;
use reprisekit::controls::addressbar::AddressBar;
use reprisekit::controls::combo::ListItem;
use reprisekit::controls::explorerbar::ExplorerBar;
use reprisekit::controls::menubar::{self, Bar, Key};
use reprisekit::controls::scrollbar::ThumbDrag;
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
use reprisekit::theme::{FontConfig, TextUnits, Theme};
use settings::Settings;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use watch::Watch;
use window::Popup;

/// Index of the Folders button in [`Explorer::toolbar`]'s items, for the latched state.
const FOLDERS_ITEM: usize = 4;
/// How far the pointer must travel from a press before it is a drag, not a click.
const DRAG_SLOP: i32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Address,
    Tree,
    View,
}

#[derive(Clone, Copy, Debug)]
pub struct Hover {
    pub on: HoverOn,
    pub at: (i32, i32),
    pub since: std::time::Instant,
}

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

pub struct Explorer {
    pub place: Place,
    pub history: History<Place>,
    pub sv: ShellView,
    sv_layout: LayoutCache,
    pub items: Vec<Item>,
    /// By path, for the thumbnails the worker hands back. Rebuilt by [`Explorer::resort`].
    item_index: HashMap<PathBuf, usize>,
    pub tree: TreeView,
    pub tree_places: Vec<Place>,
    /// The drives Computer lists. The scan stats every mount (a hung network
    /// mount blocks it), so it runs only in [`Explorer::sync_drives`].
    pub drives: Vec<reprisefs::places::Drive>,
    /// Tells a drive's root apart from a plain folder in the tree.
    pub drive_mounts: Vec<PathBuf>,
    mounts_seen: String,
    pub(crate) udisks: Option<reprisefs::udisks::Udisks>,
    /// Read with every drive scan and every Computer listing, and taken from
    /// `volume_changes`: a stick plugged in without being mounted doesn't change `/proc/mounts`.
    pub(crate) volumes: reprisefs::udisks::Snapshot,
    /// [`reprisefs::udisks::follow`]'s snapshots, each with a wake.
    volume_changes: Option<std::sync::mpsc::Receiver<reprisefs::udisks::Snapshot>>,
    /// Read once: they change on a package install, not between listings.
    pub(crate) globs: Arc<reprisefs::apps::MimeDb>,
    /// The home and document folders a listing gives their own icon, read once.
    specials: Vec<(PathBuf, &'static str)>,
    pub bar: Bar,
    pub toolbar: ToolBar,
    pub address: AddressBar,
    pub address_places: Vec<Place>,
    pub explorer_bar: ExplorerBar,
    pub taskpane: TaskPane,
    pub pane_cmds: Vec<Vec<commands::Command>>,
    pub pane_tips: Vec<Vec<Option<&'static str>>>,
    /// What the task sections were built for, so a selection change that keeps
    /// them only rewrites Details instead of refetching every row's icon.
    pane_key: Option<pane::PaneKey>,
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
    pub undo: Option<&'static str>,
    pub undos: reprisefs::undo::UndoStack,
    /// Always true: nothing can ask the compositor what the clipboard holds
    /// without pasting, so an empty paste is a no-op.
    pub can_paste: bool,
    pub paste_shortcut: bool,
    /// The folder a paste was asked for, captured then: the answer arrives an
    /// event later, when the window may have moved somewhere that takes no writes.
    pub paste_dir: Option<PathBuf>,
    /// Scanned once; Open, Open With and the chooser read it.
    pub app_index: Vec<reprisefs::apps::AppEntry>,
    /// Indices into `app_index`, parallel to the Open With and Send To rows.
    pub open_with_apps: Vec<usize>,
    pub open_with_path: Option<PathBuf>,
    pub send_to: Vec<reprisefs::sendto::SendToRow>,
    pub dialog: Option<Dialog>,
    /// The Properties sheet, in its own modeless window: it and a box can be up at once.
    pub props: Option<Props>,
    error_icon: Option<Canvas>,
    /// `children(&self)` has no `Ctx` to borrow a renderer from.
    dialog_text: TextRenderer,
    /// The dialog font's px at 100%; the folder picker and the Open With chooser stay at it.
    dialog_px: u32,
    /// The forms theme with the dialog font at the text size, which the boxes draw with and
    /// size their windows by ([`Explorer::follow_forms`]).
    forms: Theme,
    clicks: DoubleClick<usize>,
    /// For the double click that takes a list row as the default button, keyed by the form item.
    dialog_clicks: DoubleClick<Option<usize>>,
    alt_arm: AltArm,
    /// The last input was a key, not a pointer press: a dialog opened now shows keyboard cues from the start.
    keyed: bool,
    dragging: bool,
    /// A marquee drag in the view: where the press landed, what was selected
    /// then, and whether Ctrl was held (the band toggles against that set instead
    /// of replacing it). The rectangle lives in `sv.marquee`, appearing once the
    /// pointer has moved [`DRAG_SLOP`] px.
    marquee_from: Option<((i32, i32), std::collections::BTreeSet<usize>, bool)>,
    /// A Details column divider drag: the column, the press position and the column's 100% width then.
    resizing: Option<(usize, i32, i32)>,
    thumb: ThumbDrag,
    tree_thumb: ThumbDrag,
    pub drag: Option<drag::Drag>,
    pub incoming: Option<drag::Incoming>,
    pub pending_drop: Option<drag::PendingDrop>,
    pub(crate) put: Option<fileops::Put>,
    pub(crate) put_queue: std::collections::VecDeque<fileops::PendingPut>,
    /// While a replace prompt is up: the batches whose prompts come next, and
    /// the error boxes of batches that finished meanwhile.
    pub(crate) replace_waiting:
        std::collections::VecDeque<(fileops::PendingPut, reprisefs::replace::Asker)>,
    pub(crate) messages_waiting: std::collections::VecDeque<(String, String)>,
    /// Finished background jobs (deletes, the bin's Empty) waiting for a wake.
    jobs_tx: std::sync::mpsc::Sender<fileops::JobDone>,
    jobs_rx: std::sync::mpsc::Receiver<fileops::JobDone>,
    pub(crate) jobs_pending: usize,
    /// Close came while a put or job was out: the program quits at the wake that finds them all done.
    pub(crate) closing: bool,
    /// The thumbnail worker. Every listing bumps the generation, which drops
    /// work for a folder the user has left.
    thumbs: Thumbs,
    generation: u64,
    /// Image sizes by path, from the thumbnail worker or a header read
    /// ([`Explorer::read_dims`]). They survive a sort and a refresh and feed the
    /// Tiles detail line, the infotip and the Details pane.
    image_dims: HashMap<PathBuf, (u32, u32)>,
    /// Images whose header was sent to a job in this listing, so a file that states no size is read once.
    dims_asked: HashSet<PathBuf>,
    watch: Option<Watch>,
    pub hover: Option<Hover>,
    pub tip: Option<Tip>,
    pub folder_tip: Option<(PathBuf, Vec<String>)>,
    pub tip_walk: Option<live::TipWalk>,
}

impl Explorer {
    fn new(start: Place, ctx: &mut Ctx) -> Explorer {
        let settings = Settings::load();
        let apps = reprisefs::apps::scan(&reprisefs::apps::default_dirs());
        let udisks = reprisefs::udisks::Udisks::system();
        let mut ex = Explorer::with_settings(start, settings, apps, udisks, ctx);
        ex.follow_volumes(ctx.wake.clone());
        ex
    }

    /// The window on `settings`, `app_index` and `udisks` as given, for tests that read nothing of the developer's own.
    fn with_settings(
        start: Place,
        settings: Settings,
        app_index: Vec<reprisefs::apps::AppEntry>,
        udisks: Option<reprisefs::udisks::Udisks>,
        ctx: &mut Ctx,
    ) -> Explorer {
        let (jobs_tx, jobs_rx) = std::sync::mpsc::channel();
        let thumb_cache = settings
            .thumbnail_cache
            .enabled
            .then(|| thumbs::open_cache(settings.thumbnail_cache.max_bytes()))
            .flatten();
        let thumb_size = settings.thumbnail_size;
        let drives = reprisefs::places::drives();
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
        let mut ex = Explorer {
            place: start.clone(),
            history: History::new(start.clone()),
            sv,
            sv_layout: LayoutCache::default(),
            items: Vec::new(),
            item_index: HashMap::new(),
            tree: TreeView::new(Vec::new(), Style::Explorer),
            tree_places: Vec::new(),
            drive_mounts: place::drive_mounts(&drives),
            drives,
            mounts_seen: std::fs::read_to_string("/proc/mounts").unwrap_or_default(),
            volumes: reprisefs::udisks::Snapshot::default(),
            volume_changes: None,
            udisks,
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
            address: AddressBar::new(vec![ListItem::new("")], 0, None),
            address_places: Vec::new(),
            explorer_bar: ExplorerBar {
                title: "Folders".to_string(),
                width: settings.bar_width,
                hot_close: false,
                greyed: false,
            },
            taskpane: TaskPane::default(),
            pane_cmds: Vec::new(),
            pane_tips: Vec::new(),
            pane_key: None,
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
            undos: reprisefs::undo::UndoStack::default(),
            can_paste: true,
            paste_shortcut: false,
            paste_dir: None,
            app_index,
            open_with_apps: Vec::new(),
            open_with_path: None,
            send_to: Vec::new(),
            dialog: None,
            props: None,
            error_icon: None,
            dialog_text: TextRenderer::load(&FontConfig::default()),
            dialog_px: Theme::default().fonts.size_px,
            forms: Theme::default(),
            clicks: DoubleClick::default(),
            dialog_clicks: DoubleClick::default(),
            alt_arm: AltArm::default(),
            keyed: false,
            dragging: false,
            marquee_from: None,
            resizing: None,
            thumb: ThumbDrag::default(),
            tree_thumb: ThumbDrag::default(),
            drag: None,
            incoming: None,
            pending_drop: None,
            put: None,
            put_queue: std::collections::VecDeque::new(),
            replace_waiting: std::collections::VecDeque::new(),
            messages_waiting: std::collections::VecDeque::new(),
            jobs_tx,
            jobs_rx,
            jobs_pending: 0,
            closing: false,
            thumbs: Thumbs::new(
                Arc::new(Decoders::default_set()),
                thumb_size,
                thumb_cache,
                ctx.wake.clone(),
            ),
            generation: 0,
            image_dims: HashMap::new(),
            dims_asked: HashSet::new(),
            watch: match Watch::new(ctx.wake.clone()) {
                Ok(w) => Some(w),
                Err(e) => {
                    log::warn!("no folder watch: {e}");
                    None
                }
            },
            hover: None,
            tip: None,
            folder_tip: None,
            tip_walk: None,
        };
        ex.follow_forms(ctx);
        ex.build_toolbar(ctx);
        ex.build_address(ctx);
        ex.build_tree(ctx);
        ex.sync_bar();
        ex.show(start, ctx);
        ex
    }

    /// The keyboard cannot be left in a tree that is gone.
    pub fn sync_bar(&mut self) {
        if self.focus == Focus::Tree && !self.settings.folders {
            self.focus = Focus::View;
        }
    }

    pub fn layout(&self, ctx: &Ctx) -> window::Layout {
        window::layout(
            ctx.size,
            &self.settings,
            self.explorer_bar.width,
            statusbar::height(&ctx.theme.controls()),
            &ctx.theme.controls(),
        )
    }

    /// Rebuilt only when the view's geometry or the look's control sizes changed.
    pub fn view_layout(&self, frame: Rect, ctx: &Ctx) -> Rc<shellview::Layout> {
        self.sv_layout.get(&self.sv, frame, &ctx.theme.controls())
    }

    pub fn bar_layout(&self, ctx: &Ctx) -> menubar::Layout {
        let menu = self.layout(ctx).menu;
        self.bar.layout(
            &ctx.text,
            ctx.theme.fonts.size_px,
            menu,
            &ctx.theme.controls(),
        )
    }

    fn menu_state(&self) -> MenuState {
        MenuState {
            mode: self.sv.mode,
            sort: self.sv.sort.unwrap_or(self.settings.sort),
            selection: self.sv.selection.len(),
            can_paste: self.can_paste,
            can_undo: self.undo,
            folders: self.settings.folders,
            status_bar: self.settings.status_bar,
            toolbars: [self.settings.toolbars, self.settings.address_bar, false],
            read_only: self.place.read_only(),
            bin: (self.place == Place::RecycleBin).then_some(self.items.len()),
            can_back: self.history.can_back(),
            can_forward: self.history.can_forward(),
        }
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
        let fresh = menus::menu_bar(&self.menu_state());
        self.bar.titles = fresh.titles;
        self.bar.menus = fresh.menus;
    }

    /// Go somewhere new, recorded in the history. Going where we already are
    /// is a refresh: Enter on unchanged address text re-lists the folder.
    pub fn go(&mut self, place: Place, ctx: &mut Ctx) -> Reply {
        if place == self.place {
            return self.refresh(ctx);
        }
        self.history.visit(place.clone());
        self.show(place, ctx)
    }

    /// Re-list the current place, keeping the selection by path and leaving the
    /// history alone: F5, View > Refresh, and Enter on the address already shown.
    pub fn refresh(&mut self, ctx: &mut Ctx) -> Reply {
        let chosen = self.selected_paths();
        // The folder watch fires refreshes unasked; they keep the scroll position.
        let (top, left) = (self.sv.top, self.sv.left);
        // A rename in progress survives the re-listing, by path: the folder watch
        // fires one the instant New > Folder writes its folder.
        let editing = self
            .sv
            .edit
            .as_ref()
            .and_then(|(i, ed)| self.items.get(*i).map(|it| (it.path.clone(), ed.clone())));
        let here = self.place.clone();
        if here == Place::Computer {
            self.mounts_seen.clear();
        }
        let reply = self.show(here, ctx);
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

    pub fn show(&mut self, place: Place, ctx: &mut Ctx) -> Reply {
        if !self.sync_drives(ctx) && place == Place::Computer {
            self.sync_volumes();
        }
        let Listed { items, groups, .. } = list(
            &place,
            &self.globs,
            self.settings.show_hidden,
            &self.drives,
            &self.volumes,
        );
        self.set_columns(&place);
        let is_12h = reprisefs::locale::is_12h();
        // Resolved up front: `item_icon` asks what the theme carries, and the icons below want the cache mutable.
        let names: Vec<String> = items
            .iter()
            .map(|it| reprisefs::icons::item_icon(it, &self.specials, &|n| ctx.icons.has(n)))
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
        let mut icons = ListingIcons::default();
        self.sv.hot = None;
        self.sv.entries = items
            .iter()
            .enumerate()
            .map(|(i, it)| {
                let [icon, icon32, icon48] = icons.get(&mut ctx.icons, &names[i]);
                let mut e = Entry {
                    id: i as u64,
                    name: it.label().to_string(),
                    icon,
                    icon32,
                    icon48,
                    // Thumbnails arrive later from the `thumbs` worker, or are carried over below.
                    thumb: None,
                    is_dir: it.is_dir,
                    link: it.is_link || it.shortcut.is_some(),
                    cut: false,
                    size: it.size,
                    kind: it.kind.clone(),
                    modified: it
                        .modified
                        .map(|s| reprisefs::listing::date_text(s, is_12h))
                        .unwrap_or_default(),
                    mtime: it.modified,
                    detail: Vec::new(),
                    folder: if let Some(t) = &it.trashed {
                        t.original
                            .parent()
                            .map(|d| d.display().to_string())
                            .unwrap_or_default()
                    } else {
                        String::new()
                    },
                    deleted: it
                        .trashed
                        .as_ref()
                        .and_then(|t| t.deleted)
                        .map(|s| reprisefs::listing::date_text(s, is_12h))
                        .unwrap_or_default(),
                };
                e.detail = shellview::tile_detail(&e);
                e
            })
            .collect();
        let mut dims = HashMap::new();
        for (it, e) in items.iter().zip(self.sv.entries.iter_mut()) {
            let Some((size, modified, thumb)) = kept.remove(&it.path) else {
                continue;
            };
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
        if stale_sheet {
            self.props = None;
        }
        self.place = place.clone();
        self.resort();

        let icon = ctx.icons.get(place.icon_name(), 16);
        self.address.set_place(&place.address(), icon);
        self.tree_follow(&place, ctx);
        self.refresh_chrome(ctx);
        // The listing is new: every earlier thumbnail request is stale.
        self.generation += 1;
        self.thumbs.cancel_all();
        self.hide_tip();
        self.request_thumbs();
        if let Some(w) = self.watch.as_mut() {
            match place.path() {
                Some(_) if place == Place::RecycleBin => w.watch_all(&place::bin_watches()),
                Some(dir) => w.watch(&dir),
                None => w.clear(),
            }
        }
        Reply {
            redraw: true,
            redraw_all: stale_sheet,
            ..Default::default()
        }
    }

    /// Details' columns for the Recycle Bin (with Original Location and Date
    /// Deleted) or for a folder (the user's widths), switched only when the kind
    /// of listing changes. The bin keeps the mode and sorts by name without
    /// touching the saved sort.
    fn set_columns(&mut self, place: &Place) {
        let bin = *place == Place::RecycleBin;
        let want = if bin {
            shellview::bin_fields()
        } else {
            shellview::details_fields()
        };
        if self.sv.fields == want {
            return;
        }
        if bin {
            self.sv.columns = shellview::bin_columns();
            self.sv.sort = Some((0, true));
        } else {
            self.sv.columns = shellview::details_columns();
            for (col, w) in self.sv.columns.iter_mut().zip(&self.settings.columns) {
                col.width = *w;
            }
            self.sv.sort = Some(self.settings.sort);
        }
        self.sv.fields = want;
    }

    /// Column widths and the sort are saved only for a folder's own columns, never the bin's.
    pub(crate) fn folder_columns(&self) -> bool {
        self.sv.fields == shellview::details_fields()
    }

    /// Rescan the drives and UDisks2 volumes when `/proc/mounts` changed: the
    /// tree and address drop follow the mounts. Returns whether it rescanned.
    fn sync_drives(&mut self, ctx: &mut Ctx) -> bool {
        let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
        if mounts == self.mounts_seen {
            return false;
        }
        self.drives = reprisefs::places::drives();
        self.drive_mounts = place::drive_mounts(&self.drives);
        self.mounts_seen = mounts;
        self.sync_volumes();
        self.build_address(ctx);
        true
    }

    /// Read UDisks2 again and mark the mounted drives it calls removable:
    /// `/proc/mounts` alone takes an unlocked stick's cleartext volume for a fixed disk.
    fn sync_volumes(&mut self) {
        if let Some(u) = &self.udisks {
            self.volumes = u.snapshot();
        }
        for d in &mut self.drives {
            if self.volumes.removable_device(&d.device) {
                d.removable = true;
            }
        }
    }

    fn selected_dir(&self) -> Option<Place> {
        let i = self.sv.selection.single()?;
        folder_place(self.items.get(i)?)
    }

    /// In the Recycle Bin an item opens its Properties (its menu's default
    /// row): a deleted file is not opened until restored.
    pub fn open_selected(&mut self, ctx: &mut Ctx) -> Reply {
        let Some(i) = self.sv.selection.single() else {
            return Reply::none();
        };
        if self.place == Place::RecycleBin {
            return self.properties(ctx);
        }
        let Some(it) = self.items.get(i).cloned() else {
            return Reply::none();
        };
        if let Some(v) = self.selected_unmounted() {
            return self.mount_and_open(v, ctx);
        }
        if let Some(place) = folder_place(&it) {
            return self.go(place, ctx);
        }
        Reply {
            redraw: true,
            spawn: vec![self.open_argv_for(&it)],
            ..Default::default()
        }
    }

    /// Re-order by the current sort column. Every mode shows the same order, so
    /// this runs on every listing and header click. A grouped listing sorts
    /// within each group. The selection is kept by path.
    pub fn resort(&mut self) {
        let Some((col, asc)) = self.sv.sort else {
            self.index_items();
            return;
        };
        let chosen = self.selected_paths();
        let firsts: Vec<usize> = self.sv.groups.iter().map(|g| g.first).collect();
        let field = self.sv.fields.get(col).copied().unwrap_or(Field::Name);
        let order = sort_order_grouped(&self.items, &firsts, field, asc);
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

    /// The entry `--select` names. Nothing happens when the listing doesn't show it (a hidden file).
    pub(crate) fn select_shown(&mut self, path: &Path, ctx: &mut Ctx) {
        let Some(i) = self.index_of(path) else {
            return;
        };
        self.sv.selection.set_only(Some(i));
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
    start: Place,
    select: Option<PathBuf>,
    inner: Option<Explorer>,
    /// How the text grows as the program starts, for the window's opening size.
    units: TextUnits,
}

impl App for Lazy {
    fn title(&self) -> String {
        match &self.inner {
            Some(ex) => ex.place.title(),
            None => self.start.title(),
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

    fn text_sized(&self) -> bool {
        true
    }

    fn children(&self) -> Vec<Child> {
        match &self.inner {
            Some(ex) if !ex.closing => ex.children(),
            _ => Vec::new(),
        }
    }

    fn hidden(&self) -> bool {
        self.inner.as_ref().is_some_and(|ex| ex.closing)
    }

    fn caption(&self, win: WinId) -> repriseapp::Caption {
        self.inner
            .as_ref()
            .map_or(repriseapp::Caption::Window, |ex| ex.caption(win))
    }

    fn min_size(&self, win: WinId) -> Option<(i32, i32)> {
        self.inner.as_ref().and_then(|ex| ex.min_size(win))
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        if self.inner.is_none() {
            let mut ex = Explorer::new(self.start.clone(), ctx);
            if let Some(path) = self.select.take() {
                ex.select_shown(&path, ctx);
            }
            self.inner = Some(ex);
        }
        let ex = self.inner.as_mut().expect("built above");
        ex.follow_forms(ctx);
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
        if win == DIALOG {
            return ex.dialog_event(ev, ctx);
        }
        if win == PROGRAM_BROWSE {
            return ex.program_browse_event(ev, ctx);
        }
        if win == browse::ERROR {
            return ex.browse_error_event(ev);
        }
        if win == PROPS {
            return ex.props_event(ev, ctx);
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
                    && ex.dialog.is_none()
                    && ex.props.is_none()
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
            Event::Wake => ex.wake(ctx),
            Event::Blink => {
                ex.flush_settings();
                Reply::redraw()
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
                ex.alt_arm.interrupt();
                let closed = ex.close_drop_menu();
                ex.flush_settings();
                Reply {
                    redraw: closed,
                    ..Reply::none()
                }
            }
            // A maximized window's size is the screen's: a hand size still pending is saved.
            Event::Resized if ctx.maximized => Reply::redraw(),
            Event::Resized => {
                ex.window_px = (!ctx.at_pick).then_some(ctx.size);
                ex.settings_dirty = true;
                Reply::redraw()
            }
            Event::CloseRequest => ex.close(),
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

    fn drop_form(&mut self, win: WinId) -> Option<DropForm<'_>> {
        let form = match win {
            PROPS => self.inner.as_mut()?.props.as_mut()?.sheet.open_mut(),
            _ => return None,
        };
        Some(DropForm { form, px: None })
    }

    fn drop_file_dialog(&mut self, win: WinId) -> Option<FileDrop<'_>> {
        let Some(Dialog::ChooseProgram(cp)) = self.inner.as_mut()?.dialog.as_mut() else {
            return None;
        };
        let b = cp.browse.as_mut().filter(|_| win == PROGRAM_BROWSE)?;
        Some(FileDrop {
            dlg: &mut b.dlg,
            host: &mut b.host,
        })
    }

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        let Some(ex) = &mut self.inner else {
            c.fill_rect(c.bounds(), ctx.theme.colors.button_face);
            return;
        };
        ex.follow_forms(ctx);
        if win == DIALOG {
            ex.draw_dialog(c, ctx);
            return;
        }
        if win == PROGRAM_BROWSE {
            ex.draw_program_browse(c);
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
        ex.status.grip = !ctx.maximized;
        // A look that boxes its items greys the unfocused pane's selection; Classic keeps the highlight.
        let boxed = item::boxed(&ctx.theme.paint());
        ex.tree.inactive = boxed && ex.focus != Focus::Tree;
        ex.sv.inactive = boxed && ex.focus != Focus::View;
        window::draw(ex, c, ctx);
    }

    fn access(&mut self, win: WinId, tree: &mut reprisekit::access::Tree, ctx: &Ctx) {
        let Some(ex) = &mut self.inner else {
            return;
        };
        ex.follow_forms(ctx);
        match win {
            DIALOG => ex.access_dialog(tree, ctx),
            PROGRAM_BROWSE => ex.access_program_browse(tree),
            browse::ERROR => ex.access_browse_error(tree),
            PROPS => {
                if let Some(p) = &ex.props {
                    let px = ctx.forms().fonts.size_px;
                    let (_, focus) = p
                        .sheet
                        .access(tree, reprisekit::access::ROOT, &ctx.text, px);
                    if let Some(f) = focus {
                        tree.set_focus(f);
                    }
                }
            }
            _ => window::access(ex, tree, ctx),
        }
    }
}

/// `reprise-explorer [PLACE]` or `--select PATH`. PLACE is a folder's path or
/// "Computer" or "Recycle Bin"; a missing or unknown one is Home. `--select`
/// opens the folder an absolute PATH sits in with PATH selected and scrolled
/// into view (Find Target's window).
fn parse_args(args: impl IntoIterator<Item = std::ffi::OsString>) -> (Place, Option<PathBuf>) {
    let mut args = args.into_iter();
    let first = args.next();
    if first.as_deref() == Some("--select".as_ref()) {
        let path = args.next().map(PathBuf::from);
        if let Some(path) = path.filter(|p| p.is_absolute())
            && let Some(dir) = path.parent().filter(|d| d.is_dir())
        {
            return (Place::from_path(dir), Some(path));
        }
        return (Place::Home, None);
    }
    let start = first.and_then(|arg| match arg.to_str() {
        Some(s) => Place::parse_address(s, None),
        None => {
            let p = PathBuf::from(arg);
            (p.is_absolute() && p.is_dir()).then(|| Place::from_path(&p))
        }
    });
    (start.unwrap_or(Place::Home), None)
}

fn main() {
    env_logger::init();
    let (start, select) = parse_args(std::env::args_os().skip(1));
    repriseapp::run(Box::new(Lazy {
        start,
        select,
        inner: None,
        units: repriseapp::start_forms().text,
    }));
}

/// A folder itself, or the folder a symlink to one points at; None for anything else.
fn folder_place(it: &Item) -> Option<Place> {
    if !it.is_dir {
        return None;
    }
    let to = match it.is_link {
        true => std::fs::canonicalize(&it.path).unwrap_or_else(|_| it.path.clone()),
        false => it.path.clone(),
    };
    Some(Place::from_path(&to))
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
