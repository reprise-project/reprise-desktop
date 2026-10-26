//! The host side of `reprisekit::controls::filedlg` every program shares: places tiles, icons and
//! folder listings. Folder ids are handles in the program's `FolderTable`.

use std::collections::{BTreeSet, HashMap};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::{Event, PointerInput};
use reprisefs::listing::{Item, Row};
use reprisekit::controls::filedlg::DlgMode;
use reprisekit::controls::form;
use reprisekit::raster::Rect;
use reprisekit::selection::Mods;
use reprisekit::text::TextRenderer;
use reprisekit::textedit::keysym;

use crate::listicons::{ListIcons, ReadIcons};
use crate::listjob::Job;
use crate::thumbs::{self, Thumbs};
use reprisefs::folders::FolderTable;
use reprisekit::art::viewsicon;
use reprisekit::controls::filedlg::{Crumb, Fields, FileDlg, Folder, FolderId, Hit, Remembered};
use reprisekit::controls::placesbar::{self, Place};
use reprisekit::controls::shellview::{self, Entry, Mode};
use reprisekit::icontheme::{self, IconCache, IconTheme, ListingIcons};
use reprisekit::pointer::DoubleClick;
use reprisekit::raster::Canvas;
use reprisekit::theme::{Metrics, TextUnits, Theme};

fn dialog_places() -> Vec<reprisefs::places::DialogPlace> {
    reprisefs::places::dialog_places(repriseconf::recent_files())
}

pub fn place_dir(i: usize) -> Option<PathBuf> {
    dialog_places().into_iter().nth(i)?.dir
}

pub fn places() -> Vec<Place> {
    dialog_places()
        .into_iter()
        .map(|p| Place {
            enabled: p.dir.is_some(),
            label: p.label,
            icon: None,
        })
        .collect()
}

/// The toolbar's icon names in `FileDlg::set_toolbar_icons`' order (Back, Up, New Folder, Views).
/// `view-list` may not resolve; then Views gets the drawn icon ([`viewsicon::draw16_near`]).
const TOOLBAR_ICONS: [&str; 4] = ["go-previous", "go-up", "folder-new", "view-list"];
const TOOLBAR_VIEWS: usize = 3;
const TOOLBAR_PX: i32 = 16;

#[derive(Default)]
pub struct Icons {
    pub places: [Option<Canvas>; 6],
    pub toolbar: [Option<Canvas>; 4],
    /// The Look in field's folder, grown with the text like the toolbar's.
    pub folder: Option<Canvas>,
    /// The cache's scale and theme and the text size when they were fetched.
    scale: i32,
    theme: Option<Arc<IconTheme>>,
    text: TextUnits,
}

/// The drawn Views icon `px` logical px across on a canvas at `scale`, as the cache's are.
fn views_icon(px: i32, scale: i32) -> Canvas {
    viewsicon::draw16_near(px * scale).at_scale(scale)
}

impl Icons {
    /// Fetches the icons not yet held, at the text size they were last fetched at.
    pub fn prime(&mut self, icons: &mut IconCache) {
        self.prime_at(icons, self.text);
    }

    /// Fetches the icons not yet held, grown by `u` as the dialog's places bar and toolbar are;
    /// all of them again when `icons` is at another scale or theme, or `u` is another size.
    fn prime_at(&mut self, icons: &mut IconCache, u: TextUnits) {
        if !self.fetched_from(icons, u) {
            *self = Icons {
                scale: icons.scale(),
                theme: Some(icons.theme()),
                text: u,
                ..Icons::default()
            };
        }
        let get = |icons: &mut IconCache, name: &str, px: i32| {
            let icon = icons.get(name, px)?;
            Some(icontheme::grown_icon(icon, px, u))
        };
        for (icon, place) in self.places.iter_mut().zip(dialog_places()) {
            if icon.is_none() {
                *icon = get(icons, place.icon, u.len(placesbar::ICON_PX));
            }
        }
        let px = u.len(TOOLBAR_PX);
        for (i, (icon, name)) in self.toolbar.iter_mut().zip(TOOLBAR_ICONS).enumerate() {
            if icon.is_none() {
                *icon = get(icons, name, px)
                    .or_else(|| (i == TOOLBAR_VIEWS).then(|| views_icon(px, icons.scale())));
            }
        }
        if self.folder.is_none() {
            self.folder = get(icons, "folder", px);
        }
    }

    fn fetched_from(&self, icons: &IconCache, u: TextUnits) -> bool {
        self.scale == icons.scale()
            && self.text == u
            && self
                .theme
                .as_ref()
                .is_some_and(|t| Arc::ptr_eq(t, &icons.theme()))
    }

    pub fn places(&self) -> Vec<Place> {
        let mut places = places();
        for (place, icon) in places.iter_mut().zip(&self.places) {
            place.icon = icon.clone();
        }
        places
    }

    pub fn dialog(&self, fields: Fields) -> FileDlg {
        let mut dlg = FileDlg::new(fields, self.places());
        self.set_toolbar(&mut dlg);
        dlg
    }

    fn set_toolbar(&self, dlg: &mut FileDlg) {
        let [back, up, new_folder, views] = self.toolbar.clone();
        dlg.set_toolbar_icons(back, up, new_folder, views);
    }
}

struct Listing {
    folder: Folder,
    /// Names shown for paths they don't spell: non-UTF-8 names and My Recent Documents' files.
    aliases: Vec<(String, OsString)>,
    /// The pictures' paths; picture `i`'s entry has id `i + 1`.
    pictures: Vec<PathBuf>,
    icons: ListingIcons,
}

/// A folder's rows and their icons as a listing's thread read them ([`read`]).
struct Read {
    path: PathBuf,
    rows: Vec<Row>,
    icons: ReadIcons,
    /// List and Details' icon size the icon files were decoded at.
    list: i32,
}

/// Folder `path`'s rows, hidden entries and those `keep` refuses left out, with their icons read
/// at `scale` and List and Details' at `list` px. Runs on a listing's thread: any of it may sit
/// on a mount that never answers.
fn read(path: PathBuf, keep: impl FnMut(&Item) -> bool, scale: i32, list: i32) -> Read {
    let globs = reprisefs::apps::mime_globs();
    let rows = reprisefs::listing::dialog_rows(&path, &globs, keep);
    let sizes = icontheme::listing_px(list);
    let icons = ReadIcons::read(rows.iter().map(|r| &r.item), sizes, scale);
    Read {
        path,
        rows,
        icons,
        list,
    }
}

/// What a folder's read depends on besides `keep`, which follows the dialog's filter. A relist
/// with the same key while the read is in flight waits on that read instead of starting another.
#[derive(Clone, Debug, PartialEq)]
struct ReadKey {
    id: FolderId,
    folders_only: bool,
    typed: Option<String>,
    filter: usize,
    scale: i32,
    list: i32,
}

impl ReadKey {
    fn of(dlg: &FileDlg, id: FolderId, scale: i32) -> ReadKey {
        ReadKey {
            id,
            folders_only: dlg.mode() == DlgMode::Folder,
            typed: dlg.wildcard().map(str::to_string),
            filter: dlg.filter(),
            scale,
            list: shellview::list_icon_px(dlg.text()),
        }
    }
}

/// What a listing keeps: every folder, and in a file dialog the entries a `typed` pattern
/// accepts, else those `keep` does.
fn picks(
    folders_only: bool,
    typed: Option<String>,
    keep: impl Fn(&Item) -> bool,
) -> impl Fn(&Item) -> bool {
    move |i| {
        i.is_dir
            || (!folders_only
                && match &typed {
                    Some(p) => reprisekit::controls::filedlg::pattern_keeps(p, &i.name),
                    None => keep(i),
                })
    }
}

/// Folder `id`'s `read` as the dialog lists it. Entries wear Explorer's icons at every size and
/// its Tiles lines; a picture's thumbnail arrives later ([`take_thumbs`]). Every subfolder gets
/// its own handle in `folders`.
fn listing(folders: &mut FolderTable, id: FolderId, read: Read, icons: &mut IconCache) -> Listing {
    let Read {
        path,
        rows,
        icons: read_icons,
        list,
    } = read;
    read_icons.put(icons);
    let specials = reprisefs::places::special_folders();
    let names: Vec<String> = rows
        .iter()
        .zip(&read_icons.own)
        .map(|(r, own)| {
            reprisefs::icons::item_icon_with(&r.item, own.as_deref(), &specials, &|n| icons.has(n))
        })
        .collect();
    let mut shared = ListingIcons::default().with_list(list);
    let mut aliases: Vec<(String, OsString)> = Vec::new();
    let mut pictures = Vec::new();
    let entries = rows
        .into_iter()
        .zip(names)
        .map(|(r, name)| {
            let it = r.item;
            if it.path.parent() != Some(path.as_path()) {
                aliases.push((it.name.clone(), it.path.clone().into_os_string()));
            } else if let Some(raw) = it.path.file_name()
                && raw.to_str().is_none()
            {
                aliases.push((it.name.clone(), raw.to_os_string()));
            }
            let id = if it.is_dir {
                folders.id(&it.path)
            } else if it.is_image() {
                pictures.push(it.path.clone());
                pictures.len() as u64
            } else {
                0
            };
            let [icon, icon32, icon48] = shared.get(icons, &name);
            let mut e = Entry {
                id,
                icon,
                icon32,
                icon48,
                name: it.name,
                is_dir: it.is_dir,
                link: it.is_link,
                size: it.size,
                kind: it.kind,
                modified: r.modified,
                mtime: it.modified,
                ..Default::default()
            };
            e.detail = shellview::tile_detail(&e);
            e
        })
        .collect();
    let parent = reprisefs::places::dialog_parent(&path).map(|p| folders.id(p));
    let label = reprisefs::places::dialog_label(&path);
    Listing {
        folder: Folder {
            id,
            parent,
            crumbs: vec![Crumb {
                id,
                label,
                depth: 0,
                icon: None,
            }],
            entries,
        },
        aliases,
        pictures,
        icons: shared,
    }
}

/// Starts thumbnails for the file dialog in window `win` ([`crate::Extended::drop_file_dialog`]):
/// the repriseapp host calls it before each event, so no program wires them up.
pub(crate) fn thumbs_on(app: &mut dyn crate::App, win: crate::WinId, wake: &crate::Waker) {
    if let Some(f) = app.drop_file_dialog(win) {
        f.host.thumbs_on(wake);
    }
}

/// The file dialog in window `win`, if any, draws at `scale` ([`Host::follow_scale`]).
pub(crate) fn follow_scale(
    app: &mut dyn crate::App,
    win: crate::WinId,
    scale: i32,
    ctx: &mut crate::Ctx,
) {
    if let Some(f) = app.drop_file_dialog(win) {
        f.host.follow_scale(f.dlg, scale, &mut ctx.icons, &ctx.wake);
    }
}

/// Hands the file dialogs in `wins` the thumbnails and icon files that arrived: the windows
/// that show one.
pub(crate) fn take_thumbs(
    app: &mut dyn crate::App,
    wins: impl IntoIterator<Item = crate::WinId>,
    icons: &mut IconCache,
) -> Vec<crate::WinId> {
    wins.into_iter()
        .filter(|&win| {
            app.drop_file_dialog(win)
                .is_some_and(|f| f.host.take_thumbs(f.dlg, icons))
        })
        .collect()
}

pub fn type_keep(dlg: &FileDlg) -> impl Fn(&Item) -> bool + Clone + Send + 'static {
    let pattern = dlg.picked_pattern();
    move |i| {
        pattern
            .as_deref()
            .is_none_or(|p| reprisekit::controls::filedlg::pattern_keeps(p, &i.name))
    }
}

#[derive(Debug)]
pub enum Step {
    Closed,
    Redraw,
    None,
    Resize,
    Out(reprisekit::controls::filedlg::Event),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolved {
    Folder(FolderId, PathBuf),
    File(PathBuf),
}

/// The file every dialog's view and sort are kept in, once [`remember_views`] ran: a test
/// that never runs a program opens every dialog in the default view and stores nothing.
static VIEW_FILE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Dialogs opened from here on open in the view and sort the last one left, and store theirs.
/// [`crate::run`] calls it; a host that opens a dialog before that calls it first.
pub fn remember_views() {
    VIEW_FILE.get_or_init(repriseconf::filedialog::path);
}

type StartRead = Box<dyn Fn() -> Job<Read>>;

#[derive(Default)]
pub struct Host {
    pub folders: FolderTable,
    pub folder: Option<FolderId>,
    pub icons: Icons,
    clicks: DoubleClick<Option<Hit>>,
    aliases: HashMap<String, OsString>,
    thumbs: Option<Thumbs>,
    /// `[thumbnail_cache]` as the last dialog opened: what `thumbs` keeps its pictures by.
    cache: Option<repriseconf::ThumbnailCache>,
    pictures: Pictures,
    list_icons: ListIcons,
    /// The scale the dialog's places tiles and toolbar wear their icons at.
    icons_scale: i32,
    /// Wakes the program's loop when a late listing lands ([`Host::take_thumbs`]).
    wake: Option<crate::Waker>,
    /// A folder still being read after [`LIST_WAIT`]; the dialog shows the one before.
    reading: Option<(ReadKey, Job<Read>)>,
    /// The icon theme the shown listing's icons came from.
    listed_theme: Option<Arc<IconTheme>>,
    /// The shown folder read again for a new icon theme's icons ([`Host::restyle`]).
    restyling: Option<Job<Read>>,
    /// The last [`Host::list`] and how to start it again: a link it listed unfollowed that
    /// answers since (`relist`), or a gvfs metadata change in the folder (`gvfs`, a folder icon
    /// `gio set` gives), reads the folder anew.
    again: Option<(ReadKey, PathBuf, StartRead)>,
    relist: Option<reprisefs::listing::Relist>,
    gvfs: Option<reprisefs::gvfsmeta::Changes>,
    /// A typed path whose folders did not answer within [`RESOLVE_WAIT`]; the next resolve of
    /// the same path waits on it again instead of starting another probe.
    probing: Option<(PathBuf, Job<(bool, bool)>)>,
    /// The view and sort the open dialog last stored ([`Host::remember`]).
    remembered: Remembered,
    /// Where they are stored; None is [`VIEW_FILE`].
    view_file: Option<PathBuf>,
}

/// How long the dialog waits for a listing before it goes on. Tests wait as long as it takes.
const LIST_WAIT: std::time::Duration = if cfg!(test) {
    std::time::Duration::from_secs(60)
} else {
    crate::listjob::WAIT
};

/// The listed pictures and how far their thumbnails have got.
#[derive(Default)]
struct Pictures {
    /// Picture `i`'s path; its entry's id is `i + 1`.
    paths: Vec<PathBuf>,
    ids: HashMap<PathBuf, u64>,
    /// Bumped by every listing, so thumbnails for an earlier one are dropped.
    generation: u64,
    /// This listing's pictures are queued for decoding.
    asked: bool,
    /// The scale the window drew at when the thumbnails were asked for.
    scale: i32,
}

impl Host {
    /// The dialog on `dir`, laid out for text grown by `u` ([`crate::Ctx::forms`]'s): the host
    /// draws it with that theme and sizes its window from [`FileDlg::client`].
    pub fn open(&mut self, dir: &Path, fields: Fields, u: TextUnits) -> FileDlg {
        self.folder = Some(self.folders.id(dir));
        self.follow_cache(repriseconf::thumbnail_cache());
        let mut dlg = self.icons.dialog(fields);
        self.icons_scale = 0;
        dlg.set_text(u);
        self.remembered = self
            .view_file()
            .map(|f| Remembered::parse(&repriseconf::filedialog::load_from(&f)))
            .unwrap_or_default();
        dlg.restore(self.remembered);
        dlg
    }

    fn view_file(&self) -> Option<PathBuf> {
        self.view_file.clone().or_else(|| VIEW_FILE.get().cloned())
    }

    /// Stores the dialog's view and sort once they change, for every dialog opened after.
    fn remember(&mut self, dlg: &FileDlg) {
        let now = dlg.remembered();
        if now == self.remembered {
            return;
        }
        self.remembered = now;
        let Some(path) = self.view_file() else {
            return;
        };
        if let Err(e) = repriseconf::filedialog::save_to(&path, &now.text()) {
            log::warn!(
                "file dialog: could not store its view in {}: {e}",
                path.display()
            );
        }
    }

    /// Starts the worker that decodes the listed pictures for Thumbnails and Tiles, once; `wake`
    /// fires as they arrive and [`Host::take_thumbs`] shows them.
    fn thumbs_on(&mut self, wake: &crate::Waker) {
        self.wake.get_or_insert_with(|| wake.clone());
        self.relist.get_or_insert_with(|| {
            let wake = wake.clone();
            reprisefs::listing::Relist::new(move || wake.wake())
        });
        self.gvfs.get_or_insert_with(|| {
            let wake = wake.clone();
            reprisefs::gvfsmeta::Changes::new(move || wake.wake())
        });
        if self.thumbs.is_some() {
            return;
        }
        let cache = *self.cache.get_or_insert_with(repriseconf::thumbnail_cache);
        self.thumbs = Some(Thumbs::new(
            std::sync::Arc::new(crate::decode::Decoders::default_set()),
            thumbs::cache_for(cache),
            wake.clone(),
        ));
    }

    /// Stops a worker started under another `[thumbnail_cache]` than `want`; the next event
    /// starts one under `want`.
    fn follow_cache(&mut self, want: repriseconf::ThumbnailCache) {
        if self.cache.replace(want).is_some_and(|had| had != want)
            && let Some(old) = self.thumbs.take()
        {
            // Dropping joins the worker, which first decodes what the last listing queued.
            std::thread::spawn(move || drop(old));
        }
    }

    /// Queues this listing's pictures once Views shows Thumbnails or Tiles.
    fn ask_thumbs(&mut self, dlg: &FileDlg) {
        let Some(worker) = &mut self.thumbs else {
            return;
        };
        let p = &mut self.pictures;
        if p.asked || !matches!(dlg.view(), Mode::Thumbnails | Mode::Tiles) {
            return;
        }
        p.asked = true;
        for path in &p.paths {
            worker.request(path.clone(), p.generation, p.scale);
        }
    }

    /// The dialog draws at `scale`, `icons`' scale: its places, toolbar and entries wear their
    /// icons at that scale, the places and toolbar from `icons`' theme at the dialog's text size
    /// ([`FileDlg::text`]), and its pictures are
    /// asked for again at it. The thumbnails and icon files it shows stay until theirs arrive
    /// and `wake` fires.
    fn follow_scale(
        &mut self,
        dlg: &mut FileDlg,
        scale: i32,
        icons: &mut IconCache,
        wake: &crate::Waker,
    ) {
        let rescaled = std::mem::replace(&mut self.icons_scale, scale) != scale;
        if rescaled || !self.icons.fetched_from(icons, dlg.text()) {
            self.icons.prime_at(icons, dlg.text());
            dlg.set_place_icons(self.icons.places.to_vec());
            self.icons.set_toolbar(dlg);
            dlg.set_crumb_icons(vec![self.icons.folder.clone()]);
        }
        let theme = icons.theme();
        if let Some(had) = &self.listed_theme
            && !Arc::ptr_eq(had, &theme)
        {
            self.listed_theme = Some(theme);
            if let Some((_, _, start)) = &self.again {
                self.restyling = Some(start());
            }
        }
        let list = shellview::list_icon_px(dlg.text());
        self.list_icons
            .follow(scale, list, icons, dlg.entries_mut(), wake);
        let p = &mut self.pictures;
        if p.scale != scale {
            p.scale = scale;
            p.asked = false;
            self.ask_thumbs(dlg);
        }
    }

    /// Hands the thumbnails and icon files that arrived to their entries, the icon files to
    /// `cache` too: true when one did.
    fn take_thumbs(&mut self, dlg: &mut FileDlg, cache: &mut IconCache) -> bool {
        self.read_due();
        if let Some((id, read)) = self
            .reading
            .as_ref()
            .and_then(|(k, j)| Some((k.id, j.take()?)))
        {
            self.reading = None;
            self.show_read(dlg, id, read, cache);
            return true;
        }
        if let Some(read) = self.restyling.as_ref().and_then(Job::take) {
            self.restyling = None;
            self.restyle(dlg, read, cache);
            return true;
        }
        let icons = self.list_icons.take(dlg.entries_mut(), cache);
        let Some(worker) = &mut self.thumbs else {
            return icons;
        };
        let done = worker.drain();
        if done.is_empty() {
            return icons;
        }
        let at: HashMap<u64, usize> = dlg
            .entries()
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.is_dir && e.id != 0)
            .map(|(i, e)| (e.id, i))
            .collect();
        let mut shown = icons;
        for d in done {
            let Some(&i) = self.pictures.ids.get(&d.path).and_then(|id| at.get(id)) else {
                continue;
            };
            let e = &mut dlg.entries_mut()[i];
            e.thumb = Some(shellview::frame_thumb(d.thumb, d.dims, d.scale));
            e.detail = shellview::image_detail(d.dims, &e.kind);
            shown = true;
        }
        shown
    }

    pub fn event(
        &mut self,
        dlg: &mut FileDlg,
        ev: &Event,
        text: &TextRenderer,
        forms: &Theme,
        size: (i32, i32),
        metrics: &Metrics,
    ) -> Step {
        // A text size change reaches the dialog as a resize of its window, at the old size.
        if *ev == Event::Resized && dlg.text() != forms.text {
            dlg.set_text(forms.text);
            return Step::Redraw;
        }
        let step = self.event_inner(dlg, ev, text, forms.fonts.size_px, size, metrics);
        self.remember(dlg);
        self.ask_thumbs(dlg);
        step
    }

    fn event_inner(
        &mut self,
        dlg: &mut FileDlg,
        ev: &Event,
        text: &TextRenderer,
        px: u32,
        size: (i32, i32),
        metrics: &Metrics,
    ) -> Step {
        use reprisekit::controls::filedlg::Event as E;
        let (w, h) = dlg.client();
        let r = Rect::new(0, 0, w, h);
        let mods = |m: &crate::Mods| Mods {
            ctrl: m.ctrl,
            shift: m.shift,
        };
        let out = match ev {
            Event::CloseRequest => return Step::Closed,
            Event::Blink => dlg.blink(),
            Event::Resized => {
                dlg.set_size(size);
                return Step::Redraw;
            }
            Event::Key(k) => {
                // Ctrl makes no key text, so it has no edit key: read it off the keysym.
                let key = if k.mods.ctrl && k.sym == keysym::SPACE {
                    Some(form::Key::Space)
                } else {
                    crate::keys::dialog_key(k)
                };
                let Some(key) = key else {
                    return Step::None;
                };
                let ch = k.utf8.as_deref().and_then(|s| s.chars().next());
                dlg.key(r, key, ch, mods(&k.mods))
            }
            Event::Pointer(PointerInput::Press {
                x,
                y,
                button: crate::BTN_LEFT,
                mods: m,
                time,
            }) => {
                let hit = dlg.hit(text, px, r, *x, *y);
                if self.clicks.press(hit, metrics.click(*x, *y, *time)) {
                    dlg.double_click(r, hit)
                } else {
                    dlg.click(r, hit, mods(m))
                }
            }
            // File name takes the focus for the edit menu the host opens over it.
            Event::Pointer(PointerInput::Press {
                x,
                y,
                button: crate::BTN_RIGHT,
                mods: m,
                ..
            }) => match dlg.hit(text, px, r, *x, *y) {
                Some(reprisekit::controls::filedlg::Hit::FileName) => dlg.click(
                    r,
                    Some(reprisekit::controls::filedlg::Hit::FileName),
                    mods(m),
                ),
                _ => return Step::None,
            },
            Event::Pointer(PointerInput::Press { button, .. }) => {
                match reprisekit::pointer::nav_button(*button) {
                    Some(nav) => dlg.nav(nav),
                    None => return Step::None,
                }
            }
            Event::Pointer(PointerInput::Release {
                button: crate::BTN_LEFT,
                ..
            }) => dlg.release(),
            Event::Pointer(PointerInput::Motion { x, y }) => dlg.motion(text, px, r, *x, *y),
            Event::Pointer(PointerInput::Leave) => dlg.leave(),
            Event::Pointer(PointerInput::Scroll { x, y, rows }) => {
                let hit = dlg.hit(text, px, r, *x, *y);
                dlg.wheel(r, hit, *rows)
            }
            _ => return Step::None,
        };
        match out {
            E::NeedPlace(i) => match self.place(i) {
                Some(id) => Step::Out(E::NeedListing(id)),
                None => Step::None,
            },
            E::Wildcard { dir, name } => match self.wildcard_folder(&dir) {
                Some(id) => {
                    dlg.apply_wildcard(&name);
                    Step::Out(E::NeedListing(id))
                }
                None => Step::Out(E::Accept(vec![name])),
            },
            E::None => Step::None,
            E::Redraw => Step::Redraw,
            E::Resize => Step::Resize,
            out => Step::Out(out),
        }
    }

    pub fn place(&mut self, i: usize) -> Option<FolderId> {
        place_dir(i).map(|dir| self.folders.id(&dir))
    }

    fn wildcard_folder(&mut self, dir: &str) -> Option<FolderId> {
        if dir.is_empty() {
            return self.folder;
        }
        match self.resolve(dir) {
            Ok(Resolved::Folder(id, _)) => Some(id),
            _ => None,
        }
    }

    /// Lists folder `id` in `dlg`, keeping the folders and the entries `keep` (or a typed
    /// pattern) accepts; `keep` follows the dialog's filter. The folder is read on a thread; one
    /// that takes longer than [`LIST_WAIT`] lands later through [`Host::take_thumbs`], the dialog
    /// showing the folder before until then.
    pub fn list<K>(&mut self, dlg: &mut FileDlg, id: FolderId, icons: &mut IconCache, keep: &K)
    where
        K: Fn(&Item) -> bool + Clone + Send + 'static,
    {
        let Some(path) = self.folders.path(id).map(Path::to_path_buf) else {
            return;
        };
        let key = ReadKey::of(dlg, id, icons.scale());
        let start = {
            let (path, keep, scale, list) = (path.clone(), keep.clone(), key.scale, key.list);
            let (folders_only, typed) = (key.folders_only, key.typed.clone());
            let wake = self.wake.clone();
            move || {
                let (path, keep) = (
                    path.clone(),
                    picks(folders_only, typed.clone(), keep.clone()),
                );
                Job::spawn(wake.as_ref(), move || read(path, keep, scale, list))
            }
        };
        let job = match self.reading.take() {
            Some((k, job)) if k == key => job,
            _ => start(),
        };
        self.again = Some((key.clone(), path, Box::new(start)));
        match job.wait(LIST_WAIT) {
            Some(r) => self.show_read(dlg, id, r, icons),
            None => self.reading = Some((key, job)),
        }
    }

    /// Reads the last listed folder again once a link it listed unfollowed has answered or gvfs
    /// metadata changed for a file in it.
    fn read_due(&mut self) {
        let due = self.relist.as_ref().map(|r| r.take()).unwrap_or_default();
        let changed = self.gvfs.as_ref().map(|g| g.take()).unwrap_or_default();
        self.read_again(&due, &changed);
    }

    /// Reads the last listed folder again when it is one of `due` or holds one of `changed`.
    fn read_again(&mut self, due: &BTreeSet<PathBuf>, changed: &BTreeSet<PathBuf>) {
        if let Some((key, path, start)) = &self.again
            && (due.contains(path) || changed.iter().any(|f| f.parent() == Some(path)))
        {
            self.reading = Some((key.clone(), start()));
        }
    }

    fn show_read(&mut self, dlg: &mut FileDlg, id: FolderId, read: Read, icons: &mut IconCache) {
        let mut l = listing(&mut self.folders, id, read, icons);
        for c in &mut l.folder.crumbs {
            c.icon = self.icons.folder.clone();
        }
        self.folder = Some(id);
        self.aliases.clear();
        self.aliases.extend(l.aliases);
        self.list_icons.listed(l.icons, icons.scale());
        self.listed_theme = Some(icons.theme());
        self.restyling = None;
        let p = &mut self.pictures;
        p.generation += 1;
        p.asked = false;
        p.scale = icons.scale();
        p.ids = (1..)
            .zip(&l.pictures)
            .map(|(id, path)| (path.clone(), id))
            .collect();
        p.paths = l.pictures;
        if let Some(t) = &mut self.thumbs {
            t.cancel_all();
        }
        dlg.set_listing(l.folder);
        self.ask_thumbs(dlg);
    }

    /// The shown folder's `read` for a new icon theme: each entry still listed wears the icon
    /// the theme gives it, named again since the theme may lack the old name; the view, its
    /// selection and scroll stay.
    fn restyle(&mut self, dlg: &mut FileDlg, read: Read, icons: &mut IconCache) {
        let Some(id) = dlg.folder() else {
            return;
        };
        if self.folders.path(id) != Some(read.path.as_path()) {
            return;
        }
        let l = listing(&mut self.folders, id, read, icons);
        let fresh: HashMap<&str, &Entry> = l
            .folder
            .entries
            .iter()
            .map(|e| (e.name.as_str(), e))
            .collect();
        for e in dlg.entries_mut() {
            if let Some(f) = fresh.get(e.name.as_str()) {
                e.icon = f.icon.clone();
                e.icon32 = f.icon32.clone();
                e.icon48 = f.icon48.clone();
            }
        }
        self.list_icons.listed(l.icons, icons.scale());
    }

    /// Lists the dialog's folder once, unless it is still being read.
    pub fn ensure_listed<K>(&mut self, dlg: &mut FileDlg, icons: &mut IconCache, keep: &K)
    where
        K: Fn(&Item) -> bool + Clone + Send + 'static,
    {
        if dlg.folder().is_none()
            && self.reading.is_none()
            && let Some(id) = self.folder
        {
            self.list(dlg, id, icons, keep);
        }
    }

    pub fn alias(&mut self, shown: String, raw: OsString) {
        self.aliases.insert(shown, raw);
    }

    pub fn resolve(&mut self, name: &str) -> Result<Resolved, String> {
        let base = self
            .folder
            .and_then(|id| self.folders.path(id))
            .map(Path::to_path_buf)
            .ok_or_else(|| "The folder is no longer available.".to_string())?;
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let raw = match (name, &home) {
            ("~", Some(h)) => h.clone(),
            (n, Some(h)) if n.starts_with("~/") => h.join(&n[2..]),
            (n, _) => match self.aliases.get(n) {
                Some(raw) => base.join(raw),
                None if reprisefs::places::is_recent(&base) && !n.starts_with('/') => {
                    return Err(format!(
                        "{n} is not in My Recent Documents. Type the full path to save or open it elsewhere."
                    ));
                }
                None => base.join(n),
            },
        };
        let path = reprisefs::ops::lexical(&raw);
        let job = match self.probing.take() {
            Some((p, job)) if p == path => job,
            _ => probe_folders(&path),
        };
        let Some(found) = job.wait(RESOLVE_WAIT) else {
            let err = format!("{} is not responding.", path.display());
            self.probing = Some((path, job));
            return Err(err);
        };
        match found {
            (true, _) => Ok(Resolved::Folder(self.folders.id(&path), path)),
            (false, true) => Ok(Resolved::File(path)),
            (false, false) => Err(format!(
                "The folder {} does not exist.",
                path.parent().unwrap_or(&path).display()
            )),
        }
    }
}

/// How long [`Host::resolve`] waits on a typed path's folders before it calls them dead.
const RESOLVE_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// Whether `path` is a folder and whether its parent is, read on a listing thread so a mount
/// that stopped answering hangs the thread and not the loop.
fn probe_folders(path: &Path) -> Job<(bool, bool)> {
    let path = path.to_path_buf();
    Job::spawn(None, move || {
        (path.is_dir(), path.parent().is_some_and(Path::is_dir))
    })
}

pub fn taken(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

#[cfg(test)]
mod tests;
