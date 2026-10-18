//! The compositor's host for `reprisekit::controls::filedlg`: the Browse
//! dialog for Run or Open With (see `Purpose`). The picker
//! owns the `FileDlg` model, the `reprisefs::folders::FolderTable` its
//! `FolderId`s resolve through, and the icons the host looked up; a places
//! tile resolves through `place_dirs`. `dialog.rs` frames it, routes keys
//! and clicks to it and draws it into the frame's client area.
//!
//! The picker never touches the file system itself: a folder read or a
//! typed name's lookup can block for good on a dead network mount. It
//! queues a [`Request`] instead; the host runs it off the loop and hands
//! the [`Reply`] back through [`FilePicker::reply`]. Pictures' thumbnails
//! come the same way: [`FilePicker::take_pictures`], then [`FilePicker::thumbs`].

use crate::dialog::Outcome;
use crate::raster::{Canvas, Rect};
use reprisefs::listing::Row;
use reprisekit::art::viewsicon;
use reprisekit::controls::filedlg::{self, Crumb, Event, Fields, FileDlg, Folder, FolderId};
use reprisekit::controls::form::Key;
use reprisekit::controls::placesbar::Place;
use reprisekit::controls::shellview::{self, Entry, Mode};
use reprisekit::frame::Edges;
use reprisekit::icontheme::{IconCache, ListingIcons};
use reprisekit::selection::Mods;
use reprisekit::text::TextRenderer;
use reprisekit::textedit::EditKey;
use std::path::{Path, PathBuf};

pub const TOOLBAR: [&str; 4] = ["go-previous", "go-up", "folder-new", "view-list"];
/// Few themes carry `view-list`: without it Views gets the drawn icon at the dialog's scale.
const TOOLBAR_VIEWS: usize = 3;

/// What the picker is for: which filters it offers, which files it lists,
/// and where the pick goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// Run's Browse...: programs (and every file behind the
    /// second filter); the pick lands quoted in Run's field.
    RunCommand,
    /// The Open With chooser's Browse...: the same listing, the pick
    /// joining the chooser's list.
    Program,
}

impl Purpose {
    fn fields(self) -> Fields {
        let filters = vec!["Programs".into(), "All Files".into()];
        Fields {
            title: "Browse".into(),
            accept_label: "Open".into(),
            filters,
            mode: filedlg::DlgMode::Open,
            multiple: false,
        }
    }

    /// The files the current filter lists: None is everything.
    fn exts(self, filter: usize) -> Option<&'static [&'static str]> {
        match (self, filter) {
            // "Programs" is what the shell would run: executable files, judged by
            // mode rather than by extension (`listing::Item::exec`).
            (Purpose::RunCommand | Purpose::Program, 0) => Some(&[]),
            (Purpose::RunCommand | Purpose::Program, _) => None,
        }
    }
}

/// The dialog's own icons, fetched by the host: the places tiles at 32 px in
/// `reprisefs::places::dialog_places`' order, the toolbar's buttons at 16 in `TOOLBAR`'s, and the
/// Look in folder. The dialog draws on device pixels, so each is a scale-1 canvas `scale` times
/// that size, as are the listing's, which come from the cache handed to [`FilePicker::reply`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Icons {
    pub places: Vec<Option<Canvas>>,
    pub toolbar: Vec<Option<Canvas>>,
    pub folder: Option<Canvas>,
    pub scale: i32,
}

/// The dialog's icons from the compositor's scale-1 `cache`, for a dialog drawn at `scale`.
pub fn icons(cache: &mut IconCache, scale: i32) -> Icons {
    let scale = scale.max(1);
    Icons {
        places: reprisefs::places::dialog_places(true)
            .iter()
            .map(|p| cache.get(p.icon, 32 * scale))
            .collect(),
        toolbar: TOOLBAR
            .iter()
            .map(|i| cache.get(i, 16 * scale))
            .enumerate()
            .map(|(n, icon)| match icon {
                None if n == TOOLBAR_VIEWS => Some(viewsicon::draw16_at(scale).at_scale(1)),
                icon => icon,
            })
            .collect(),
        folder: cache.get("folder", 16 * scale),
        scale,
    }
}

/// The listing's icon sizes: List and Details, Icons and Thumbnails, Tiles.
const LISTING_PX: [i32; 3] = [16, 32, 48];

/// The Browse dialog: the dialog, its folder table and icons.
#[derive(Clone, Debug, PartialEq)]
pub struct FilePicker {
    pub dlg: FileDlg,
    purpose: Purpose,
    /// Where each places tile points; None is a greyed tile.
    place_dirs: Vec<Option<PathBuf>>,
    folders: reprisefs::folders::FolderTable,
    icons: Icons,
    /// The Files of type filter the listing was built for; a change
    /// relists the folder.
    filter: usize,
    /// Requests made since the host last took them.
    requests: Vec<Request>,
    /// Requests made and not yet answered. A request equal to one of them takes its number
    /// instead of another job, so relisting a folder that never answers strands no more threads.
    pending: Vec<Request>,
    /// The last request's number: every request takes the next.
    seq: u64,
    /// The folder listing still wanted and the typed name still being
    /// looked up, by request number. A reply for anything else is stale.
    listing: Option<u64>,
    opening: Option<u64>,
    /// The relisting of the folder shown that only gives its entries their icons at the
    /// dialog's new scale ([`FilePicker::follow_scale`]).
    restyling: Option<u64>,
    /// My Recent Documents' files by the name each is listed as, while
    /// that listing is on screen.
    recent: std::collections::HashMap<String, PathBuf>,
    pictures: Pictures,
    /// The view and sort last stored, and the file they live in once
    /// [`FilePicker::remember_in`] named it.
    remembered: filedlg::Remembered,
    view_file: Option<PathBuf>,
}

/// The listed pictures and how far their thumbnails have got.
#[derive(Clone, Debug, Default, PartialEq)]
struct Pictures {
    /// Picture `i`'s path; its entry's id is `i + 1`.
    paths: Vec<PathBuf>,
    /// Bumped by every listing, so thumbnails for an earlier one are dropped.
    generation: u64,
    /// This listing's pictures went to the host.
    asked: bool,
    /// The scale the dialog drew at when listed: the thumbnails' device pixels.
    scale: i32,
}

/// File system work the picker is waiting on, for the host to run off
/// the loop with [`Request::run`].
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    seq: u64,
    /// The folder to list, or None to look `path` up first: a typed name
    /// is a folder to enter, a file to pick, or nothing.
    id: Option<FolderId>,
    path: PathBuf,
    exts: Option<&'static [&'static str]>,
    /// The pattern typed into File name, which lists in place of `exts`.
    pattern: Option<String>,
    /// The lookup is a typed pattern's folder part: a folder is entered
    /// and the dialog takes `pattern`; anything else is no pick.
    folder_only: bool,
    /// The dialog's draw scale, at which icon files the listing names are decoded.
    scale: i32,
}

impl Request {
    /// Asks for the same file system work, whatever its number.
    fn same_work(&self, o: &Request) -> bool {
        self.id == o.id
            && self.scale == o.scale
            && self.path == o.path
            && self.exts == o.exts
            && self.pattern == o.pattern
            && self.folder_only == o.folder_only
    }

    /// Reads the folder or looks the name up. Blocks for as long as the
    /// file system does.
    pub fn run(self) -> Reply {
        let pattern = self.pattern.as_deref();
        let found = if self.id.is_some() {
            Found::Folder(read_folder(&self.path, self.exts, pattern))
        } else {
            match std::fs::metadata(&self.path) {
                Ok(m) if m.is_dir() => Found::Folder(read_folder(&self.path, self.exts, pattern)),
                Ok(m) if m.is_file() && !self.folder_only => Found::File,
                _ => Found::Missing,
            }
        };
        let art = match &found {
            Found::Folder(rows) => icon_files(rows, self.scale),
            _ => Vec::new(),
        };
        Reply {
            seq: self.seq,
            scale: self.scale,
            id: self.id,
            path: self.path,
            pattern: self.folder_only.then_some(self.pattern).flatten(),
            found,
            art,
        }
    }
}

/// A listed row and, for a folder, its own icon (`.directory`'s `Icon=`), read on the job's
/// thread: a subfolder may be a mount point that never answers.
#[derive(Debug)]
struct Listed {
    row: Row,
    own_icon: Option<String>,
}

/// What a [`Request`] found, for [`FilePicker::reply`].
#[derive(Debug)]
pub struct Reply {
    seq: u64,
    /// The scale its icons were made for.
    scale: i32,
    id: Option<FolderId>,
    path: PathBuf,
    /// The typed pattern whose folder part was looked up, which the
    /// dialog takes once that is a folder.
    pattern: Option<String>,
    found: Found,
    /// Icon files the listing's rows wear (a folder's or a launcher's `Icon=/...`), decoded at
    /// [`LISTING_PX`] times `scale` on the job's thread: the file may be anywhere, a dead mount
    /// included.
    art: Vec<(String, [Option<Canvas>; 3])>,
}

#[derive(Debug)]
enum Found {
    /// `listing::Row` rather than `Entry`, which holds `Rc` icons and so
    /// cannot come back from the thread that read the folder.
    Folder(Vec<Listed>),
    File,
    Missing,
}

impl FilePicker {
    /// Run's Browse..., listing `start` (the home folder). Its Open puts
    /// the file in Run's field.
    pub fn run_command(start: &Path, icons: Icons, recent: bool) -> FilePicker {
        FilePicker::new(Purpose::RunCommand, start, icons, recent)
    }

    /// The Open With chooser's Browse..., listing `start` (`/usr/bin`).
    /// Its Open adds the program to the chooser.
    pub fn program(start: &Path, icons: Icons, recent: bool) -> FilePicker {
        FilePicker::new(Purpose::Program, start, icons, recent)
    }

    /// `recent` is `[privacy] recent_files`: without it the My Recent
    /// Documents tile is greyed.
    fn new(purpose: Purpose, start: &Path, mut icons: Icons, recent: bool) -> FilePicker {
        icons.scale = icons.scale.max(1);
        let tiles = reprisefs::places::dialog_places(recent);
        let places = tiles
            .iter()
            .enumerate()
            .map(|(i, p)| Place {
                enabled: p.dir.is_some(),
                label: p.label.clone(),
                icon: icons.places.get(i).cloned().flatten(),
            })
            .collect();
        let mut dlg = FileDlg::new(purpose.fields(), places);
        let tb = |i: usize| icons.toolbar.get(i).cloned().flatten();
        dlg.set_toolbar_icons(tb(0), tb(1), tb(2), tb(3));
        let mut picker = FilePicker {
            dlg,
            purpose,
            place_dirs: tiles.into_iter().map(|p| p.dir).collect(),
            folders: reprisefs::folders::FolderTable::default(),
            icons,
            filter: 0,
            requests: Vec::new(),
            pending: Vec::new(),
            seq: 0,
            listing: None,
            opening: None,
            restyling: None,
            recent: Default::default(),
            pictures: Pictures::default(),
            remembered: Default::default(),
            view_file: None,
        };
        let id = picker.folders.id(start);
        picker.list(id);
        picker
    }

    /// The file system work queued since the last call, for the host to
    /// run off the loop.
    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    /// Queue a listing of `id`, or a lookup of `path`. `typed` is a
    /// typed pattern whose folder part `path` is, listed through in place
    /// of the dialog's own filter.
    fn request(&mut self, id: Option<FolderId>, path: PathBuf, typed: Option<&str>) -> u64 {
        let mut req = Request {
            seq: 0,
            id,
            path,
            exts: self.purpose.exts(self.filter),
            pattern: typed.or(self.dlg.wildcard()).map(str::to_string),
            folder_only: typed.is_some(),
            scale: self.icons.scale,
        };
        if let Some(same) = self.pending.iter().find(|p| p.same_work(&req)) {
            return same.seq;
        }
        self.seq += 1;
        req.seq = self.seq;
        self.pending.push(req.clone());
        self.requests.push(req);
        self.seq
    }

    /// A finished [`Request`]: the listing lands, its entries wearing `cache`'s icons and its
    /// thumbnails sized for a dialog drawn at `scale`, or a typed name enters its folder or is the
    /// pick. A reply that a later navigation made stale changes nothing.
    pub fn reply(&mut self, r: Reply, cache: &mut IconCache, scale: i32) -> Outcome {
        self.pending.retain(|p| p.seq != r.seq);
        cache.put_files(
            r.art
                .iter()
                .flat_map(|(file, art)| {
                    LISTING_PX
                        .into_iter()
                        .zip(art)
                        .map(|(px, c)| (file.clone(), px * r.scale, c.clone()))
                })
                .collect(),
        );
        if self.restyling.is_some_and(|seq| seq == r.seq) {
            self.restyling = None;
            if let Found::Folder(rows) = r.found {
                self.restyle(rows, cache, r.scale);
            }
            return Outcome::Redraw;
        }
        if r.id.is_some() {
            if self.listing != Some(r.seq) {
                return Outcome::Nothing;
            }
            self.listing = None;
        } else {
            if self.opening != Some(r.seq) {
                return Outcome::Nothing;
            }
            self.opening = None;
        }
        match r.found {
            Found::Folder(rows) => {
                // A typed folder is newer than any listing still pending.
                self.listing = None;
                let id = r.id.unwrap_or_else(|| self.folders.id(&r.path));
                self.recent = if reprisefs::places::is_recent(&r.path) {
                    rows.iter()
                        .map(|l| (l.row.item.name.clone(), l.row.item.path.clone()))
                        .collect()
                } else {
                    Default::default()
                };
                let (entries, pictures) = self.entries(rows, cache, r.scale);
                self.show(id, &r.path, entries);
                if r.scale != self.icons.scale {
                    self.restyling = Some(self.request(Some(id), r.path.clone(), None));
                }
                self.pictures.paths = pictures;
                self.pictures.scale = scale.max(1);
                match (&r.pattern, r.id) {
                    (Some(p), _) => self.dlg.apply_wildcard(p),
                    // The field empties once the folder is entered.
                    (None, None) => self.dlg.set_file_name(""),
                    (None, Some(_)) => {}
                }
                Outcome::Redraw
            }
            Found::File => match self.purpose {
                Purpose::RunCommand => Outcome::RunPick(r.path),
                Purpose::Program => Outcome::ProgramPick(r.path),
            },
            Found::Missing => {
                log::warn!("browse: {} is not a file", r.path.display());
                Outcome::Nothing
            }
        }
    }

    /// The listing's generation, the device pixels its thumbnails fill and its pictures, once,
    /// when Views shows Thumbnails or Tiles: the host fetches their thumbnails off the loop and
    /// hands them to [`FilePicker::thumbs`].
    pub fn take_pictures(&mut self) -> Option<(u64, u32, Vec<PathBuf>)> {
        let p = &mut self.pictures;
        let shown = matches!(self.dlg.view(), Mode::Thumbnails | Mode::Tiles);
        if p.asked || !shown || p.paths.is_empty() {
            return None;
        }
        p.asked = true;
        Some((p.generation, shellview::thumb_px(p.scale), p.paths.clone()))
    }

    /// The dialog draws at `scale`: its icons are fetched at that scale from `cache`, and the
    /// listing's again off the loop; thumbnails made at another scale are dropped, since they
    /// would draw at the wrong size, and its pictures are fetched again. True when it changed.
    pub fn follow_scale(&mut self, scale: i32, cache: &mut IconCache) -> bool {
        let mut changed = false;
        if self.icons.scale != scale {
            self.icons = icons(cache, scale);
            self.dlg.set_place_icons(self.icons.places.clone());
            let tb = |i: usize| self.icons.toolbar.get(i).cloned().flatten();
            self.dlg.set_toolbar_icons(tb(0), tb(1), tb(2), tb(3));
            self.dlg.set_crumb_icons(vec![self.icons.folder.clone()]);
            if self.listing.is_none()
                && let Some(id) = self.dlg.folder()
                && let Some(path) = self.folders.path(id).map(Path::to_path_buf)
            {
                self.restyling = Some(self.request(Some(id), path, None));
            }
            changed = true;
        }
        let p = &mut self.pictures;
        if p.scale == scale || p.paths.is_empty() {
            return changed;
        }
        p.scale = scale;
        p.generation += 1;
        p.asked = false;
        for e in self.dlg.entries_mut() {
            e.thumb = None;
        }
        true
    }

    /// The folder shown, relisted at `scale`: each entry still listed takes its icons. Showing
    /// another folder drops the relisting.
    fn restyle(&mut self, rows: Vec<Listed>, cache: &mut IconCache, scale: i32) {
        let (fresh, _) = self.entries(rows, cache, scale);
        let by_name: std::collections::HashMap<&str, &Entry> =
            fresh.iter().map(|e| (e.name.as_str(), e)).collect();
        for e in self.dlg.entries_mut() {
            if let Some(f) = by_name.get(e.name.as_str()) {
                e.icon = f.icon.clone();
                e.icon32 = f.icon32.clone();
                e.icon48 = f.icon48.clone();
            }
        }
    }

    /// Thumbnails fetched for listing `generation`: each picture shows it, and its Tiles lines
    /// its size. True when one did; a listing replaced meanwhile takes none.
    pub fn thumbs(&mut self, generation: u64, done: Vec<(PathBuf, Canvas, (u32, u32))>) -> bool {
        if generation != self.pictures.generation {
            return false;
        }
        let at: std::collections::HashMap<u64, usize> = self
            .dlg
            .entries()
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.is_dir && e.id != 0)
            .map(|(i, e)| (e.id, i))
            .collect();
        let ids: std::collections::HashMap<&Path, usize> = (1..)
            .zip(&self.pictures.paths)
            .filter_map(|(id, p)| Some((p.as_path(), *at.get(&id)?)))
            .collect();
        let mut shown = false;
        for (path, thumb, dims) in done {
            let Some(&i) = ids.get(path.as_path()) else {
                continue;
            };
            let e = &mut self.dlg.entries_mut()[i];
            // The dialog draws on device pixels, so the thumbnail is a scale-1 canvas that size.
            let thumb = shellview::frame_thumb(thumb, dims, self.pictures.scale).at_scale(1);
            e.thumb = Some(thumb);
            e.detail = shellview::image_detail(dims, &e.kind);
            shown = true;
        }
        shown
    }

    pub fn current(&self) -> Option<&Path> {
        self.dlg.folder().and_then(|id| self.folders.path(id))
    }

    /// Enters `id`'s folder, empty until its listing arrives.
    fn list(&mut self, id: FolderId) {
        let Some(path) = self.folders.path(id).map(Path::to_path_buf) else {
            return;
        };
        self.show(id, &path, Vec::new());
        self.listing = Some(self.request(Some(id), path, None));
        // A folder picked after a name was typed wins over the name.
        self.opening = None;
    }

    /// `rows` as the dialog's entries, with Explorer's icons and Tiles lines, except that a
    /// program wears the executable's icon; a `folders` handle on every subfolder so entering
    /// one resolves, and [`Pictures`]' id on every picture. Returns the pictures' paths too.
    fn entries(
        &mut self,
        rows: Vec<Listed>,
        cache: &mut IconCache,
        scale: i32,
    ) -> (Vec<Entry>, Vec<PathBuf>) {
        let specials = reprisefs::places::special_folders();
        let names: Vec<String> = rows
            .iter()
            .map(|l| match &l.row.item {
                it if it.exec && !it.is_dir => "application-x-executable".to_string(),
                it => {
                    reprisefs::icons::item_icon_with(it, l.own_icon.as_deref(), &specials, &|n| {
                        cache.has(n)
                    })
                }
            })
            .collect();
        let mut shared = ListingIcons::device(scale);
        let mut pictures = Vec::new();
        let entries = rows
            .into_iter()
            .zip(names)
            .map(|(Listed { row: r, .. }, name)| {
                let it = r.item;
                let [icon, icon32, icon48] = shared.get(cache, &name);
                let id = if it.is_dir {
                    self.folders.id(&it.path)
                } else if it.is_image() {
                    pictures.push(it.path.clone());
                    pictures.len() as u64
                } else {
                    0
                };
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
        (entries, pictures)
    }

    /// Hands folder `id` at `path` to the dialog with `entries`, its pictures' thumbnails
    /// still to come.
    fn show(&mut self, id: FolderId, path: &Path, entries: Vec<Entry>) {
        self.restyling = None;
        let generation = self.pictures.generation + 1;
        self.pictures = Pictures {
            generation,
            ..Default::default()
        };
        let parent = reprisefs::places::dialog_parent(path).map(|p| self.folders.id(p));
        let label = reprisefs::places::dialog_label(path);
        self.dlg.set_listing(Folder {
            id,
            parent,
            crumbs: vec![Crumb {
                id,
                label,
                depth: 0,
                icon: self.icons.folder.clone(),
            }],
            entries,
        });
    }

    /// A key from the dialog view. Typed text goes in as an edit so the
    /// File name field takes it, and its first character doubles as the
    /// shell view's type-ahead. `client` is the dialog's page rect, where
    /// `draw` laid it out.
    pub fn key(&mut self, client: Rect, key: Key) -> Outcome {
        let k = match key {
            Key::Backspace => Key::Edit(EditKey::Backspace { word: false }),
            k => k,
        };
        let ch = match &k {
            Key::Edit(EditKey::Insert(t)) => t.chars().next(),
            Key::Char(c) => Some(*c),
            _ => None,
        };
        let ev = self.dlg.key(client, k, ch, Mods::default());
        self.event(ev)
    }

    /// A mouse's thumb button: Back or Forward through the folders shown.
    pub fn nav(&mut self, nav: reprisekit::pointer::Nav) -> Outcome {
        let ev = self.dlg.nav(nav);
        self.event(ev)
    }

    pub fn click(&mut self, client: Rect, hit: Option<filedlg::Hit>) -> Outcome {
        let ev = self.dlg.click(client, hit, Mods::default());
        self.event(ev)
    }

    /// A reader asked to see entry `row` of the list at client point `at`; nothing is picked.
    pub fn reveal(
        &mut self,
        client: Rect,
        at: (i32, i32),
        row: usize,
        text: &TextRenderer,
        px: u32,
    ) -> Outcome {
        let ev = self.dlg.reveal(text, px, client, at, row);
        self.event(ev)
    }

    /// A reader's Focus on the control at `at`: it takes the keyboard, nothing is pressed.
    pub fn focus_at(
        &mut self,
        client: Rect,
        at: (i32, i32),
        text: &TextRenderer,
        px: u32,
    ) -> Outcome {
        let ev = self.dlg.focus_at(text, px, client, at);
        self.event(ev)
    }

    /// The wheel turned `rows` rows (down is positive) over `hit`.
    pub fn wheel(&mut self, client: Rect, hit: Option<filedlg::Hit>, rows: i32) -> Outcome {
        let ev = self.dlg.wheel(client, hit, rows);
        self.event(ev)
    }

    /// The second press of a double click: a folder is entered, a file
    /// picked; anywhere but an entry it is one more click.
    pub fn double_click(&mut self, client: Rect, hit: Option<filedlg::Hit>) -> Outcome {
        let ev = self.dlg.double_click(client, hit);
        self.event(ev)
    }

    /// A changed Files of type filter relists the folder for it.
    fn sync_filter(&mut self) {
        let filter = self.dlg.filter();
        if filter != self.filter {
            self.filter = filter;
            if let Some(id) = self.dlg.folder() {
                self.list(id);
            }
        }
    }

    pub fn release(&mut self) -> Outcome {
        let ev = self.dlg.release();
        self.event(ev)
    }

    /// The pointer rests on `hit`, or drags a held column title there: a
    /// Details column title lights or follows. True when that redraws.
    pub fn hover(&mut self, hit: Option<filedlg::Hit>) -> bool {
        !matches!(self.dlg.hover(hit), Event::None)
    }

    /// The pointer left the dialog: no column title stays lit. True when
    /// that redraws.
    pub fn leave(&mut self) -> bool {
        !matches!(self.dlg.leave(), Event::None)
    }

    /// Opens in the view and sort `file` holds (`repriseconf::filedialog`), the one every Open
    /// and Save As dialog shares, and stores the dialog's own there as they change.
    pub fn remember_in(&mut self, file: PathBuf) {
        self.remembered = filedlg::Remembered::parse(&repriseconf::filedialog::load_from(&file));
        self.dlg.restore(self.remembered);
        self.view_file = Some(file);
    }

    fn remember(&mut self) {
        let now = self.dlg.remembered();
        let Some(file) = self.view_file.clone().filter(|_| now != self.remembered) else {
            return;
        };
        self.remembered = now;
        crate::state::run_in_order(move || {
            if let Err(e) = repriseconf::filedialog::save_to(&file, &now.text()) {
                log::warn!(
                    "browse: could not store the view in {}: {e}",
                    file.display()
                );
            }
        });
    }

    /// What the dialog asked for. Open on a name looks it up off the loop:
    /// a folder is entered, a file is the pick, anything else ignored.
    fn event(&mut self, ev: Event) -> Outcome {
        self.sync_filter();
        self.remember();
        let ev = match ev {
            Event::NeedPlace(i) => match self.place_dirs.get(i).cloned().flatten() {
                Some(dir) => Event::NeedListing(self.folders.id(&dir)),
                None => Event::None,
            },
            ev => ev,
        };
        match ev {
            Event::None => Outcome::Nothing,
            Event::Redraw => Outcome::Redraw,
            Event::NeedPlace(_) => Outcome::Nothing,
            Event::NeedListing(id) => {
                // Entering a folder by its typed name empties the field;
                // a name typed for something else stays.
                let typed = self.dlg.file_name().to_string();
                let names_dir = self
                    .dlg
                    .entries()
                    .iter()
                    .any(|e| e.is_dir && e.name == typed);
                self.list(id);
                if names_dir {
                    self.dlg.set_file_name("");
                }
                Outcome::Redraw
            }
            Event::CreateFolder(_) => Outcome::Nothing,
            Event::Resize => Outcome::Size(Edges::BOTTOM | Edges::RIGHT),
            Event::FilterChanged(_) => Outcome::Redraw,
            // A typed pattern lists the open folder, or looks its folder
            // part up and enters it, through the pattern. The dialog
            // takes the pattern only once there is a folder to list.
            Event::Wildcard { dir, name } => {
                if dir.is_empty() {
                    if let Some(id) = self.dlg.folder() {
                        self.dlg.apply_wildcard(&name);
                        self.list(id);
                    }
                    return Outcome::Redraw;
                }
                let Some(path) = self.typed_path(&dir) else {
                    return Outcome::Nothing;
                };
                let pattern = &name[dir.len()..];
                self.opening = Some(self.request(None, path, Some(pattern)));
                Outcome::Nothing
            }
            Event::Cancel => Outcome::Close,
            Event::Accept(names) => {
                let Some(name) = names.into_iter().next() else {
                    return Outcome::Nothing;
                };
                let Some(path) = self.typed_path(&name) else {
                    return Outcome::Nothing;
                };
                self.opening = Some(self.request(None, path, None));
                Outcome::Nothing
            }
        }
    }

    /// Where a typed `name` points from the open folder. Lexical, so a
    /// typed ".." gets the same folder handle as Up and its parent is the
    /// one above it. None for a relative name in My Recent Documents,
    /// which is no folder to look a new name up in.
    fn typed_path(&self, name: &str) -> Option<PathBuf> {
        Some(match self.current() {
            Some(dir) if reprisefs::places::is_recent(dir) => match self.recent.get(name) {
                Some(p) => p.clone(),
                None if name.starts_with('/') => reprisefs::ops::lexical(Path::new(name)),
                None => return None,
            },
            Some(dir) => reprisefs::ops::lexical(&dir.join(name)),
            None => reprisefs::ops::lexical(Path::new(name)),
        })
    }
}

/// The shared-mime-info globs every listing's Type column reads, parsed
/// on the first listing and kept with their description memo for the
/// session.
static GLOBS: std::sync::LazyLock<reprisefs::apps::MimeDb> =
    std::sync::LazyLock::new(reprisefs::apps::mime_globs);

/// `dir`'s subfolders and its files: those a typed `pattern` keeps,
/// else those with an extension in `exts`, the executable ones for an
/// empty `exts`, all of them for `None`, as `listing::dialog_rows` orders
/// them.
fn read_folder(dir: &Path, exts: Option<&[&str]>, pattern: Option<&str>) -> Vec<Listed> {
    let rows = reprisefs::listing::dialog_rows(dir, &GLOBS, |i| {
        i.is_dir
            || match (pattern, exts) {
                (Some(p), _) => filedlg::pattern_keeps(p, &i.name),
                (None, None) => true,
                (None, Some([])) => i.exec,
                (None, Some(exts)) => has_ext(&i.path, exts),
            }
    });
    rows.into_iter()
        .map(|row| {
            let own_icon = reprisefs::icons::own_icon(&row.item);
            Listed { row, own_icon }
        })
        .collect()
}

/// The icon files `rows` wear, each decoded once at every listing size times `scale`.
fn icon_files(rows: &[Listed], scale: i32) -> Vec<(String, [Option<Canvas>; 3])> {
    let mut files: Vec<&str> = rows
        .iter()
        .filter_map(|l| match &l.row.item.shortcut {
            Some(sc) => Some(sc.icon()),
            None => l.own_icon.as_deref(),
        })
        .filter(|name| name.starts_with('/'))
        .collect();
    files.sort_unstable();
    files.dedup();
    files
        .into_iter()
        .map(|f| {
            let art = LISTING_PX
                .map(|px| reprisekit::icontheme::decode_file(Path::new(f), px * scale, 1));
            (f.to_string(), art)
        })
        .collect()
}

fn has_ext(p: &Path, exts: &[&str]) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const R: Rect = Rect::new(0, 0, filedlg::CLIENT.0, filedlg::CLIENT.1);

    fn cache() -> IconCache {
        IconCache::new(reprisekit::icontheme::IconTheme::load("test", &[]))
    }

    #[test]
    fn views_without_a_theme_icon_draws_at_the_dialogs_scale() {
        for scale in [1, 2] {
            let views = icons(&mut cache(), scale).toolbar[TOOLBAR_VIEWS]
                .clone()
                .expect("the drawn Views icon");
            assert_eq!(
                (views.width, views.height, views.scale()),
                (16 * scale as u32, 16 * scale as u32, 1)
            );
            assert_eq!(
                views.pixels,
                viewsicon::draw16_at(scale).pixels,
                "scale {scale}"
            );
        }
    }

    /// Runs the picker's queued requests the way the host's thread would,
    /// returning the last outcome that did something.
    fn settle(p: &mut FilePicker) -> Outcome {
        settle_with(p, &mut cache())
    }

    fn settle_with(p: &mut FilePicker, icons: &mut IconCache) -> Outcome {
        let mut out = Outcome::Nothing;
        for r in p.take_requests() {
            match p.reply(r.run(), icons, 1) {
                Outcome::Nothing => {}
                o => out = o,
            }
        }
        out
    }

    fn names(p: &FilePicker) -> Vec<&str> {
        p.dlg.entries().iter().map(|e| e.name.as_str()).collect()
    }

    /// A scratch folder of this process's own: test runs of other checkouts
    /// share the temp dir.
    fn sandbox(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("reprise-filepick-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Pictures")).expect("mkdir");
        for f in ["dunes", ".hidden", "Pictures/meadow"] {
            std::fs::write(dir.join(f), "#!/bin/sh\n").expect("write");
            std::fs::set_permissions(dir.join(f), std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
        }
        std::fs::write(dir.join("notes.txt"), "x").expect("write");
        std::fs::set_permissions(
            dir.join("notes.txt"),
            std::fs::Permissions::from_mode(0o644),
        )
        .expect("chmod");
        dir
    }

    /// A typed ".." or "a/../b" enters the folder it names, whose Up is
    /// the folder above it.
    #[test]
    fn a_typed_dot_dot_enters_the_folder_above() {
        let dir = sandbox("dotdot");
        let mut p = FilePicker::run_command(&dir.join("Pictures"), Icons::default(), true);
        settle(&mut p);
        p.dlg.set_file_name("..");
        p.key(R, Key::Enter);
        assert_eq!(settle(&mut p), Outcome::Redraw);
        assert_eq!(p.current(), Some(dir.as_path()));
        assert_eq!(names(&p), ["Pictures", "dunes"]);
        p.click(R, Some(filedlg::Hit::ToolBar(1)));
        assert_eq!(p.current(), dir.parent());

        p.dlg
            .set_file_name(&format!("{}/Pictures/../Pictures", dir.display()));
        p.key(R, Key::Enter);
        settle(&mut p);
        assert_eq!(p.current(), Some(dir.join("Pictures").as_path()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn browse_shares_the_view_the_last_file_dialog_stored() {
        let dir = sandbox("remember");
        let file = dir.join("state").join("filedialog.toml");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(
            &file,
            "view = \"details\"\nsort = \"size\"\nascending = false\n",
        )
        .unwrap();
        let mut p = FilePicker::run_command(&dir, Icons::default(), true);
        p.remember_in(file.clone());
        assert_eq!(p.dlg.view(), Mode::Details);
        p.click(R, Some(filedlg::Hit::ToolBar(3)));
        p.click(R, Some(filedlg::Hit::ViewsItem(0)));
        crate::state::flush();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "view = \"thumbnails\"\nsort = \"size\"\nascending = false\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn typing_fills_the_file_name() {
        let dir = sandbox("type");
        let mut p = FilePicker::run_command(&dir, Icons::default(), true);
        p.key(R, Key::Edit(EditKey::Insert("D".into())));
        p.key(R, Key::Edit(EditKey::Insert("u".into())));
        assert_eq!(p.dlg.file_name(), "Du");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Run's Browse lists executables under "Programs", everything under
    /// "All Files", and hands the pick to Run.
    #[test]
    fn run_browse_lists_programs_and_hands_the_pick_to_run() {
        let dir = sandbox("run");
        std::fs::write(dir.join("tool"), "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(dir.join("tool"), std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut p = FilePicker::run_command(&dir, Icons::default(), true);
        settle(&mut p);
        assert_eq!(names(&p), ["Pictures", "dunes", "tool"]);
        // Files of type -> All Files: Down on the focused combo picks it.
        p.key(R, Key::Tab { back: false });
        p.key(R, Key::Down);
        settle(&mut p);
        assert_eq!(names(&p), ["Pictures", "dunes", "notes.txt", "tool"]);
        p.dlg.set_file_name("tool");
        p.key(R, Key::Enter);
        assert_eq!(settle(&mut p), Outcome::RunPick(dir.join("tool")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A typed pattern lists what it matches and every folder; one with
    /// a folder part enters that folder, the field keeping the pattern.
    /// One whose folder part is no folder leaves the dialog as it was.
    #[test]
    fn a_typed_pattern_filters_the_listing() {
        let dir = sandbox("wildcard");
        let mut p = FilePicker::run_command(&dir, Icons::default(), true);
        settle(&mut p);
        p.dlg.set_file_name("*.txt");
        p.key(R, Key::Enter);
        assert_eq!(settle(&mut p), Outcome::Redraw);
        assert_eq!(names(&p), ["Pictures", "notes.txt"]);
        p.dlg.set_file_name("Pictures/m*");
        p.key(R, Key::Enter);
        assert_eq!(settle(&mut p), Outcome::Redraw);
        assert_eq!(p.current(), Some(dir.join("Pictures").as_path()));
        assert_eq!(names(&p), ["meadow"]);
        assert_eq!(p.dlg.file_name(), "m*");
        p.dlg.set_file_name("../dunes/*");
        p.key(R, Key::Enter);
        assert_eq!(
            settle(&mut p),
            Outcome::Nothing,
            "a file is no folder to list"
        );
        assert_eq!(p.dlg.file_name(), "../dunes/*", "the field as typed");
        assert_eq!(p.dlg.wildcard(), Some("m*"), "the filter stays");
        assert_eq!(names(&p), ["meadow"], "the listing stays");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A places tile enters the folder `place_dirs` gives it; a tile
    /// index with none leaves the folder on screen.
    #[test]
    fn a_places_tile_enters_its_folder() {
        let dir = sandbox("place");
        let mut p = FilePicker::run_command(&dir, Icons::default(), true);
        assert_eq!(p.event(Event::NeedPlace(6)), Outcome::Nothing);
        assert_eq!(p.current(), Some(dir.as_path()));
        assert_eq!(p.event(Event::NeedPlace(5)), Outcome::Redraw);
        assert_eq!(p.current(), Some(Path::new("/")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file picked in My Recent Documents is the file at its own path.
    #[test]
    fn a_recent_file_picks_its_own_path() {
        let dir = sandbox("recent");
        let mut p = FilePicker::run_command(&dir, Icons::default(), true);
        settle(&mut p);
        let recent = p.folders.id(Path::new(reprisefs::places::RECENT));
        p.list(recent);
        p.take_requests();
        let file = dir.join("dunes");
        p.reply(
            Reply {
                seq: p.listing.unwrap(),
                scale: 1,
                id: Some(recent),
                path: PathBuf::from(reprisefs::places::RECENT),
                pattern: None,
                found: Found::Folder(read_folder(&dir, Some(&[]), None)),
                art: Vec::new(),
            },
            &mut cache(),
            1,
        );
        assert!(!p.dlg.can_go_up(), "Recent has no folder above it");
        p.dlg.set_file_name("nope");
        p.key(R, Key::Enter);
        assert!(
            p.take_requests().is_empty(),
            "a name not listed goes nowhere"
        );
        p.dlg.set_file_name("dunes");
        p.key(R, Key::Enter);
        let reqs = p.take_requests();
        assert_eq!(reqs[0].path, file);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_double_click_enters_a_folder() {
        let dir = sandbox("double");
        let mut p = FilePicker::run_command(&dir, Icons::default(), true);
        settle(&mut p);
        let hit = Some(filedlg::Hit::Shell(
            reprisekit::controls::shellview::Hit::Entry(0),
        ));
        p.double_click(R, hit);
        settle(&mut p);
        assert_eq!(p.current(), Some(dir.join("Pictures").as_path()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Entering a folder shows it empty at once and reads it only when
    /// the host runs the request, so a folder that never answers leaves
    /// the dialog working.
    #[test]
    fn a_folder_shows_empty_until_its_listing_lands() {
        let dir = sandbox("pending");
        let mut p = FilePicker::run_command(&dir, Icons::default(), true);
        assert_eq!(p.current(), Some(dir.as_path()));
        assert_eq!(names(&p), Vec::<&str>::new());
        assert_eq!(settle(&mut p), Outcome::Redraw);
        assert_eq!(names(&p), ["Pictures", "dunes"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A listing that lands after the user moved on is dropped: the
    /// folder picked last stays on screen.
    #[test]
    fn a_stale_listing_is_dropped() {
        let dir = sandbox("stale");
        let mut p = FilePicker::run_command(&dir, Icons::default(), true);
        settle(&mut p);
        p.dlg.set_file_name("Pictures");
        p.key(R, Key::Enter);
        settle(&mut p);
        // Back to `dir`, then into Pictures again before `dir`'s listing
        // lands.
        let back = p.dlg.nav(reprisekit::pointer::Nav::Back);
        p.event(back);
        let slow = p.take_requests();
        let pictures = p.folders.id(&dir.join("Pictures"));
        p.event(Event::NeedListing(pictures));
        assert_eq!(settle(&mut p), Outcome::Redraw);
        for r in slow {
            assert_eq!(
                p.reply(r.run(), &mut cache(), 1),
                Outcome::Nothing,
                "stale reply changes nothing"
            );
        }
        assert_eq!(p.current(), Some(dir.join("Pictures").as_path()));
        assert_eq!(names(&p), ["meadow"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Relisting a folder whose listing has not answered asks for no second job: the first
    /// one's reply fills the dialog, so a folder on a dead mount strands one thread, not one per
    /// relist.
    #[test]
    fn relisting_a_folder_still_being_read_queues_no_second_read() {
        let dir = sandbox("relist");
        let mut p = FilePicker::run_command(&dir, Icons::default(), true);
        let first = p.take_requests();
        assert_eq!(first.len(), 1);
        let here = p.dlg.folder().unwrap();
        p.event(Event::NeedListing(here));
        p.event(Event::NeedListing(here));
        assert!(
            p.take_requests().is_empty(),
            "the pending read serves the relists"
        );
        for r in first {
            assert_eq!(p.reply(r.run(), &mut cache(), 1), Outcome::Redraw);
        }
        assert_eq!(names(&p), ["Pictures", "dunes"]);
        p.event(Event::NeedListing(here));
        assert_eq!(p.take_requests().len(), 1, "an answered read is read again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A subfolder's own icon, and an icon file it names, are read with the listing on the
    /// job's thread: the reply wears them even when the folders and the file are gone by the
    /// time the listing lands, so the loop never opens them (a dead mount would hang it).
    #[test]
    fn a_folders_own_icon_comes_with_the_listing() {
        let dir = sandbox("own-icon");
        let (folder, music, cover) = (0xff20_40c0, 0xffc0_4020, 0xff20_a040);
        let mut icons = flat_icons(
            &dir.join(".icons"),
            &[("folder", folder), ("folder-music", music)],
        );
        let art = dir.join("Art");
        let pic = dir.join("cover.png");
        image::RgbaImage::from_pixel(48, 48, image::Rgba([0x20, 0xa0, 0x40, 0xff]))
            .save(&pic)
            .unwrap();
        std::fs::create_dir(&art).unwrap();
        reprisefs::meta::set_folder_icon(&dir.join("Pictures"), Some("folder-music")).unwrap();
        reprisefs::meta::set_folder_icon(&art, pic.to_str()).unwrap();
        let mut p = FilePicker::run_command(&dir, Icons::default(), true);
        let replies: Vec<Reply> = p.take_requests().into_iter().map(Request::run).collect();
        std::fs::remove_dir_all(dir.join("Pictures")).unwrap();
        std::fs::remove_dir_all(&art).unwrap();
        std::fs::remove_file(&pic).unwrap();
        for r in replies {
            p.reply(r, &mut icons, 1);
        }
        let worn = |name: &str| {
            let e = p.dlg.entries().iter().find(|e| e.name == name).unwrap();
            [&e.icon, &e.icon32, &e.icon48].map(|c| c.as_ref().map(|c| c.get(8, 8)))
        };
        assert_eq!(
            worn("Pictures"),
            [Some(music); 3],
            "Pictures wears folder-music"
        );
        assert_eq!(worn("Art"), [Some(cover); 3], "Art wears its picture");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn icons_take_the_dialogs_draw_scale_and_follow_a_new_one() {
        let dir = sandbox("icons-2x");
        let mut icons = flat_icons(&dir.join(".icons"), &[("folder", 0xff20_40c0)]);
        let art = dir.join("Art");
        let pic = dir.join("cover.png");
        image::RgbaImage::from_pixel(48, 48, image::Rgba([0x20, 0xa0, 0x40, 0xff]))
            .save(&pic)
            .unwrap();
        std::fs::create_dir(&art).unwrap();
        reprisefs::meta::set_folder_icon(&art, pic.to_str()).unwrap();
        let opened = super::icons(&mut icons, 2);
        let size = |c: &Option<Canvas>| c.as_ref().map(|c| (c.width, c.scale()));
        assert_eq!(
            size(&opened.folder),
            Some((32, 1)),
            "device pixels on a scale-1 canvas"
        );
        let mut p = FilePicker::run_command(&dir, opened, true);
        settle_with(&mut p, &mut icons);
        let widths = |p: &FilePicker, name: &str| {
            let e = p.dlg.entries().iter().find(|e| e.name == name).unwrap();
            [&e.icon, &e.icon32, &e.icon48].map(|c| c.as_ref().map(|c| c.width))
        };
        let at = |k: u32| [Some(16 * k), Some(32 * k), Some(48 * k)];
        assert_eq!(widths(&p, "Pictures"), at(2));
        assert_eq!(
            widths(&p, "Art"),
            at(2),
            "an icon file decoded off the loop"
        );

        assert!(p.follow_scale(1, &mut icons));
        assert_eq!(size(&p.icons.folder), Some((16, 1)));
        assert_eq!(widths(&p, "Art"), at(2), "until the relisting lands");
        assert_eq!(settle_with(&mut p, &mut icons), Outcome::Redraw);
        assert_eq!(widths(&p, "Pictures"), at(1));
        assert_eq!(widths(&p, "Art"), at(1));
        assert!(!p.follow_scale(1, &mut icons));
        assert!(p.take_requests().is_empty(), "nothing to relist");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An icon theme drawing `names` in their flat colours at 16, 32 and 48 px under `root`.
    fn flat_icons(root: &Path, names: &[(&str, u32)]) -> IconCache {
        let theme = root.join("flat");
        let dirs = ["16x16/apps", "32x32/apps", "48x48/apps"];
        let mut index = format!("[Icon Theme]\nName=Flat\nDirectories={}\n", dirs.join(","));
        for (d, px) in dirs.iter().zip([16, 32, 48]) {
            index += &format!("\n[{d}]\nSize={px}\nType=Fixed\n");
            std::fs::create_dir_all(theme.join(d)).unwrap();
            for (name, argb) in names {
                let [a, r, g, b] = argb.to_be_bytes();
                image::RgbaImage::from_pixel(px, px, image::Rgba([r, g, b, a]))
                    .save(theme.join(d).join(format!("{name}.png")))
                    .unwrap();
            }
        }
        std::fs::write(theme.join("index.theme"), index).unwrap();
        IconCache::new(reprisekit::icontheme::IconTheme::load(
            "flat",
            &[root.to_path_buf()],
        ))
    }

    /// Entries carry their icon at all three sizes and Tiles' lines: a program wears the
    /// executable's icon, any other file its type's.
    #[test]
    fn entries_wear_their_icons_at_every_size_and_tiles_lines() {
        let dir = sandbox("tiles");
        let (folder, program, text) = (0xff20_40c0, 0xffc0_4020, 0xff20_a040);
        let mut icons = flat_icons(
            &dir.join(".icons"),
            &[
                ("folder", folder),
                ("application-x-executable", program),
                ("text-x-generic", text),
            ],
        );
        let mut p = FilePicker::run_command(&dir, Icons::default(), true);
        p.dlg.set_filter(1);
        p.sync_filter();
        settle_with(&mut p, &mut icons);
        let entry = |name: &str| {
            p.dlg
                .entries()
                .iter()
                .find(|e| e.name == name)
                .cloned()
                .expect(name)
        };
        let worn = |e: &Entry| {
            [&e.icon, &e.icon32, &e.icon48].map(|c| c.as_ref().map(|c| (c.width, c.get(8, 8))))
        };
        let sizes = |argb| [Some((16, argb)), Some((32, argb)), Some((48, argb))];
        let pictures = entry("Pictures");
        assert_eq!(worn(&pictures), sizes(folder));
        assert!(pictures.detail.is_empty(), "a folder's tile names it alone");
        let dunes = entry("dunes");
        assert_eq!(worn(&dunes), sizes(program));
        assert_eq!(dunes.detail, [dunes.kind.clone(), "1 KB".into()]);
        let notes = entry("notes.txt");
        assert_eq!(worn(&notes), sizes(text));
        assert_eq!(notes.detail, [notes.kind.clone(), "1 KB".into()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn fnv(c: &Canvas, r: Rect) -> u64 {
        let px = (r.y..r.bottom()).flat_map(|y| (r.x..r.right()).map(move |x| c.get(x, y)));
        px.fold(0xcbf2_9ce4_8422_2325, |h, p| {
            (h ^ u64::from(p)).wrapping_mul(0x0100_0000_01b3)
        })
    }

    /// Where `argb` lands in `c`: its bounding box.
    fn found(c: &Canvas, argb: u32) -> Option<Rect> {
        let mut hit: Option<(i32, i32, i32, i32)> = None;
        for y in 0..c.height as i32 {
            for x in 0..c.width as i32 {
                if c.get(x, y) == argb {
                    let b = hit.get_or_insert((x, y, x, y));
                    *b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
                }
            }
        }
        hit.map(|(l, t, r, b)| Rect::new(l, t, r - l + 1, b - t + 1))
    }

    /// At scale 2 the pictures are fetched for the frame's 188 device pixels and fill it there.
    #[test]
    fn a_scale_2_listing_draws_its_thumbnails_at_the_frames_device_size() {
        const SAND: u32 = 0xffd2_b48c;
        let dir = sandbox("thumbs-2x");
        let pic = dir.join("dunes.png");
        std::fs::write(&pic, b"never decoded here").unwrap();
        let mut p = FilePicker::run_command(&dir, Icons::default(), true);
        p.dlg.set_filter(1);
        p.sync_filter();
        for r in p.take_requests() {
            p.reply(r.run(), &mut cache(), 2);
        }
        p.click(R, Some(filedlg::Hit::ToolBar(3)));
        p.click(R, Some(filedlg::Hit::ViewsItem(0)));
        let (generation, px, _) = p.take_pictures().expect("Thumbnails fetches the pictures");
        assert_eq!(px, 188, "the frame's inside at scale 2");
        let large = Canvas::from_pixels(256, 128, vec![SAND; 256 * 128]);
        assert!(p.thumbs(generation, vec![(pic.clone(), large, (800, 400))]));

        let theme = reprisekit::theme::Theme::default().scaled(2);
        let text = reprisekit::text::TextRenderer::load(&theme.fonts);
        let mut c = Canvas::new(R.width as u32 * 2, R.height as u32 * 2);
        p.dlg
            .draw(&mut c, &text, &theme.paint(), R, theme.fonts.size_px);
        let drawn = found(&c, SAND).expect("the thumbnail is drawn");
        assert_eq!((drawn.width, drawn.height), (188, 94));
        let frame = Rect::new(drawn.x - 2, drawn.y - 49, 192, 192);
        assert_eq!(c.get(frame.x, frame.y), theme.colors.button_face.argb());
        assert_eq!(fnv(&c, frame), 0x262c_b03d_ad0d_133d, "the scale-2 frame");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_draw_scale_fetches_the_pictures_again_at_its_size() {
        let dir = sandbox("thumbs-rescale");
        let pic = dir.join("dunes.png");
        std::fs::write(&pic, b"never decoded here").unwrap();
        let mut p = FilePicker::run_command(&dir, Icons::default(), true);
        p.dlg.set_filter(1);
        p.sync_filter();
        settle(&mut p);
        p.click(R, Some(filedlg::Hit::ToolBar(3)));
        p.click(R, Some(filedlg::Hit::ViewsItem(0)));
        let (generation, px, _) = p.take_pictures().expect("Thumbnails fetches the pictures");
        assert_eq!(px, 94);
        let thumb = |w| vec![(pic.clone(), Canvas::new(w, w / 2), (800, 400))];
        assert!(p.thumbs(generation, thumb(94)));
        assert!(!p.follow_scale(1, &mut cache()), "the scale it fetched at");

        assert!(p.follow_scale(2, &mut cache()));
        let shown = |p: &FilePicker| p.dlg.entries().iter().any(|e| e.thumb.is_some());
        assert!(!shown(&p), "a scale-1 thumbnail would draw at half size");
        let (again, px, paths) = p.take_pictures().expect("fetched again");
        assert_eq!((px, paths), (188, vec![pic.clone()]));
        assert!(!p.thumbs(generation, thumb(94)), "fetched at the old scale");
        assert!(p.thumbs(again, thumb(188)));
        let e = p.dlg.entries().iter().find(|e| e.thumb.is_some()).unwrap();
        assert_eq!(e.thumb.as_ref().unwrap().device_size(), (188, 94));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Thumbnails asks for the listing's pictures once; their thumbnails fill the frames, at the
    /// size that fits there, and give the picture's size to Tiles. A relisted folder drops the
    /// earlier listing's.
    #[test]
    fn thumbnails_fetch_the_pictures_once_and_show_their_thumbnails() {
        const SAND: u32 = 0xffd2_b48c;
        let dir = sandbox("thumbs");
        let pic = dir.join("dunes.png");
        std::fs::write(&pic, b"never decoded here").unwrap();
        let mut p = FilePicker::run_command(&dir, Icons::default(), true);
        p.dlg.set_filter(1);
        p.sync_filter();
        settle(&mut p);
        assert_eq!(p.take_pictures(), None, "List fetches nothing");
        p.click(R, Some(filedlg::Hit::ToolBar(3)));
        p.click(R, Some(filedlg::Hit::ViewsItem(0)));
        assert_eq!(p.dlg.view(), Mode::Thumbnails);
        let (generation, px, paths) = p.take_pictures().expect("Thumbnails fetches the pictures");
        assert_eq!(paths, std::slice::from_ref(&pic));
        assert_eq!(px, 94, "the frame's inside at scale 1");
        assert_eq!(p.take_pictures(), None, "once per listing");
        let thumb = || {
            vec![(
                pic.clone(),
                Canvas::from_pixels(128, 64, vec![SAND; 128 * 64]),
                (800, 400),
            )]
        };
        assert!(!p.thumbs(generation + 1, thumb()), "not this listing's");
        assert!(p.thumbs(generation, thumb()));
        let e = p
            .dlg
            .entries()
            .iter()
            .find(|e| e.name == "dunes.png")
            .unwrap();
        let t = e.thumb.as_ref().expect("the thumbnail");
        assert_eq!((t.width, t.height), (94, 47));
        assert_eq!(e.detail, ["800 x 400".to_string(), e.kind.clone()]);

        let theme = reprisekit::theme::Theme::default();
        let text = reprisekit::text::TextRenderer::load(&theme.fonts);
        let mut c = Canvas::new(R.width as u32, R.height as u32);
        p.dlg
            .draw(&mut c, &text, &theme.paint(), R, theme.fonts.size_px);
        let drawn = found(&c, SAND).expect("the thumbnail is drawn");
        assert_eq!((drawn.width, drawn.height), (94, 47));
        // The picture's cell: its frame and name. The Look in and places labels name the
        // test's folder and user.
        let cell = Rect::new(drawn.x - 9, drawn.y - 33, 112, 136);
        if reprisekit::text::test_tahoma() {
            assert_eq!(fnv(&c, cell), 0xdb3b_8c32_df81_6bd0, "Browse in Thumbnails");
        }

        p.event(Event::NeedListing(p.dlg.folder().unwrap()));
        settle(&mut p);
        let (relisted, ..) = p.take_pictures().expect("the new listing's pictures");
        assert!(relisted > generation);
        assert!(!p.thumbs(generation, thumb()), "the earlier listing's");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
