//! The host side of `reprisekit::controls::filedlg` every program shares: places tiles, icons and
//! folder listings. Folder ids are handles in the program's `FolderTable`.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::{Event, PointerInput};
use reprisefs::listing::Item;
use reprisekit::controls::filedlg::DlgMode;
use reprisekit::controls::form;
use reprisekit::raster::Rect;
use reprisekit::selection::Mods;
use reprisekit::text::TextRenderer;
use reprisekit::textedit::keysym;

use crate::listicons::ListIcons;
use crate::thumbs::{self, Thumbs};
use reprisefs::folders::FolderTable;
use reprisekit::controls::filedlg::{Crumb, Fields, FileDlg, Folder, FolderId, Hit};
use reprisekit::controls::placesbar::Place;
use reprisekit::controls::shellview::{self, Entry, Mode};
use reprisekit::icontheme::{IconCache, ListingIcons};
use reprisekit::pointer::DoubleClick;
use reprisekit::raster::Canvas;
use reprisekit::theme::{Metrics, TextUnits};

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
/// `view-list` may not resolve; a `None` gets the dialog's drawn Views icon.
const TOOLBAR_ICONS: [&str; 4] = ["go-previous", "go-up", "folder-new", "view-list"];

#[derive(Default)]
pub struct Icons {
    pub places: [Option<Canvas>; 6],
    pub toolbar: [Option<Canvas>; 4],
}

impl Icons {
    pub fn prime(&mut self, icons: &mut IconCache) {
        for (icon, place) in self.places.iter_mut().zip(dialog_places()) {
            if icon.is_none() {
                *icon = icons.get(place.icon, 32);
            }
        }
        for (icon, name) in self.toolbar.iter_mut().zip(TOOLBAR_ICONS) {
            if icon.is_none() {
                *icon = icons.get(name, 16);
            }
        }
    }

    pub fn places(&self) -> Vec<Place> {
        let mut places = places();
        for (place, icon) in places.iter_mut().zip(&self.places) {
            place.icon = icon.clone();
        }
        places
    }

    pub fn dialog(&self, fields: Fields) -> FileDlg {
        let [back, up, new_folder, views] = self.toolbar.clone();
        let mut dlg = FileDlg::new(fields, self.places());
        dlg.set_toolbar_icons(back, up, new_folder, views);
        dlg
    }
}

/// The listing of folder `id` for `FileDlg::set_listing`, hidden entries left out; `None` when `id`
/// does not resolve. Every subfolder gets its own handle in `folders`.
pub fn listing(
    folders: &mut FolderTable,
    id: FolderId,
    icons: &mut IconCache,
    folders_only: bool,
    keep: &dyn Fn(&Item) -> bool,
) -> Option<Folder> {
    listing_raw(folders, id, icons, folders_only, keep).map(|l| l.folder)
}

struct Listing {
    folder: Folder,
    /// Names shown for paths they don't spell: non-UTF-8 names and My Recent Documents' files.
    aliases: Vec<(String, OsString)>,
    /// The pictures' paths; picture `i`'s entry has id `i + 1`.
    pictures: Vec<PathBuf>,
    icons: ListingIcons,
}

/// Entries wear Explorer's icons at every size and its Tiles lines; a picture's thumbnail
/// arrives later ([`take_thumbs`]).
fn listing_raw(
    folders: &mut FolderTable,
    id: FolderId,
    icons: &mut IconCache,
    folders_only: bool,
    keep: &dyn Fn(&Item) -> bool,
) -> Option<Listing> {
    let path = folders.path(id)?.to_path_buf();
    let globs = reprisefs::apps::mime_globs();
    let rows = reprisefs::listing::dialog_rows(&path, &globs, |i| {
        if folders_only {
            i.is_dir
        } else {
            i.is_dir || keep(i)
        }
    });
    let specials = reprisefs::places::special_folders();
    let names: Vec<String> = rows
        .iter()
        .map(|r| reprisefs::icons::item_icon(&r.item, &specials, &|n| icons.has(n)))
        .collect();
    let mut shared = ListingIcons::default();
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
    Some(Listing {
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
    })
}

/// Starts thumbnails for the file dialog in window `win` ([`crate::App::drop_file_dialog`]):
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
) -> Vec<crate::WinId> {
    wins.into_iter()
        .filter(|&win| {
            app.drop_file_dialog(win)
                .is_some_and(|f| f.host.take_thumbs(f.dlg))
        })
        .collect()
}

pub fn type_keep(dlg: &FileDlg) -> impl Fn(&Item) -> bool + 'static {
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
}

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
        dlg.set_text(u);
        dlg
    }

    /// Starts the worker that decodes the listed pictures for Thumbnails and Tiles, once; `wake`
    /// fires as they arrive and [`Host::take_thumbs`] shows them.
    fn thumbs_on(&mut self, wake: &crate::Waker) {
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

    /// The dialog draws at `scale`, `icons`' scale: its entries wear their icons at that scale,
    /// and its pictures are asked for again at it. The thumbnails and icon files it shows stay
    /// until theirs arrive and `wake` fires.
    fn follow_scale(
        &mut self,
        dlg: &mut FileDlg,
        scale: i32,
        icons: &mut IconCache,
        wake: &crate::Waker,
    ) {
        self.list_icons
            .follow(scale, icons, dlg.entries_mut(), wake);
        let p = &mut self.pictures;
        if p.scale != scale {
            p.scale = scale;
            p.asked = false;
            self.ask_thumbs(dlg);
        }
    }

    /// Hands the thumbnails and icon files that arrived to their entries: true when one did.
    fn take_thumbs(&mut self, dlg: &mut FileDlg) -> bool {
        let icons = self.list_icons.take(dlg.entries_mut());
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
        px: u32,
        size: (i32, i32),
        metrics: &Metrics,
    ) -> Step {
        let step = self.event_inner(dlg, ev, text, px, size, metrics);
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
            Event::Blink => return Step::Redraw,
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

    pub fn list(
        &mut self,
        dlg: &mut FileDlg,
        id: FolderId,
        icons: &mut IconCache,
        keep: &dyn Fn(&Item) -> bool,
    ) {
        let folders_only = dlg.mode() == DlgMode::Folder;
        let typed = dlg.wildcard().map(str::to_string);
        let keep = |i: &Item| match &typed {
            Some(p) => reprisekit::controls::filedlg::pattern_keeps(p, &i.name),
            None => keep(i),
        };
        if let Some(l) = listing_raw(&mut self.folders, id, icons, folders_only, &keep) {
            self.folder = Some(id);
            self.aliases.clear();
            self.aliases.extend(l.aliases);
            self.list_icons.listed(l.icons, icons.scale());
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
    }

    pub fn ensure_listed(
        &mut self,
        dlg: &mut FileDlg,
        icons: &mut IconCache,
        keep: &dyn Fn(&Item) -> bool,
    ) {
        if dlg.folder().is_none()
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
        if path.is_dir() {
            return Ok(Resolved::Folder(self.folders.id(&path), path));
        }
        match path.parent() {
            Some(p) if p.is_dir() => Ok(Resolved::File(path)),
            _ => Err(format!(
                "The folder {} does not exist.",
                path.parent().unwrap_or(&path).display()
            )),
        }
    }
}

pub fn taken(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::controls::filedlg::CLIENT;

    fn host_in(t: &Path) -> Host {
        let mut h = Host::default();
        h.folder = Some(h.folders.id(t));
        h
    }

    fn tmp(tag: &str) -> PathBuf {
        let t = std::env::temp_dir().join(format!("repriseapp-host-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(t.join("sub")).unwrap();
        std::fs::write(t.join("a.txt"), "").unwrap();
        t
    }

    fn key(sym: u32, ctrl: bool, utf8: Option<&str>) -> Event {
        let mods = crate::Mods {
            ctrl,
            ..Default::default()
        };
        Event::Key(crate::KeyInput {
            sym,
            utf8: utf8.map(str::to_string),
            mods,
            edit: reprisekit::textedit::key_for(sym, ctrl, false, utf8),
        })
    }

    #[test]
    fn thumb_buttons_step_back_and_forward() {
        use reprisekit::controls::filedlg::{Crumb, Event as E, Folder};
        let listing = |id| Folder {
            id,
            parent: None,
            crumbs: vec![Crumb {
                id,
                label: "Folder".into(),
                depth: 0,
                icon: None,
            }],
            entries: vec![],
        };
        let mut h = Host::default();
        let mut dlg = h.icons.dialog(Fields {
            title: String::new(),
            accept_label: String::new(),
            filters: vec![],
            mode: DlgMode::Open,
            multiple: false,
        });
        dlg.set_listing(listing(1));
        dlg.set_listing(listing(2));
        let text = TextRenderer::load(&reprisekit::theme::FontConfig::default());
        let thumb = |button| {
            Event::Pointer(PointerInput::Press {
                x: 0,
                y: 0,
                button,
                time: 0,
                mods: crate::Mods::default(),
            })
        };
        assert!(matches!(
            h.event(
                &mut dlg,
                &thumb(0x113),
                &text,
                11,
                CLIENT,
                &Metrics::default()
            ),
            Step::Out(E::NeedListing(1))
        ));
        dlg.set_listing(listing(1));
        assert!(matches!(
            h.event(
                &mut dlg,
                &thumb(0x114),
                &text,
                11,
                CLIENT,
                &Metrics::default()
            ),
            Step::Out(E::NeedListing(2))
        ));
    }

    #[test]
    fn ctrl_space_toggles_a_file_into_a_multiple_pick() {
        use reprisekit::controls::filedlg::{Crumb, Folder, Hit};
        use reprisekit::controls::shellview;
        let mut dlg = FileDlg::new(
            Fields {
                title: "Open".into(),
                accept_label: "Open".into(),
                filters: vec!["All Files".into()],
                mode: DlgMode::Open,
                multiple: true,
            },
            vec![],
        );
        let file = |name: &str| Entry {
            name: name.into(),
            ..Default::default()
        };
        dlg.set_listing(Folder {
            id: 1,
            parent: None,
            crumbs: vec![Crumb {
                id: 1,
                label: "Folder".into(),
                depth: 0,
                icon: None,
            }],
            entries: vec![file("a.txt"), file("b.txt")],
        });
        let (w, h) = dlg.client();
        let r = Rect::new(0, 0, w, h);
        dlg.click(
            r,
            Some(Hit::Shell(shellview::Hit::Entry(0))),
            Mods::default(),
        );
        let text = TextRenderer::load(&reprisekit::theme::FontConfig::default());
        let mut host = Host::default();
        host.event(
            &mut dlg,
            &key(keysym::DOWN, true, None),
            &text,
            11,
            CLIENT,
            &Metrics::default(),
        );
        assert_eq!(dlg.file_name(), "a.txt", "Ctrl+Down moves the focus alone");
        host.event(
            &mut dlg,
            &key(keysym::SPACE, true, Some(" ")),
            &text,
            11,
            CLIENT,
            &Metrics::default(),
        );
        assert_eq!(dlg.file_name(), "\"b.txt\" \"a.txt\"");
    }

    #[test]
    fn resolve_a_plain_name_in_the_open_folder() {
        let t = tmp("plain");
        assert_eq!(
            host_in(&t).resolve("new.txt"),
            Ok(Resolved::File(t.join("new.txt")))
        );
        std::fs::remove_dir_all(&t).ok();
    }

    #[test]
    fn resolve_a_folder_name_opens_it() {
        let t = tmp("folder");
        let mut h = host_in(&t);
        match h.resolve("sub") {
            Ok(Resolved::Folder(id, p)) => {
                assert_eq!(p, t.join("sub"));
                assert_eq!(h.folders.path(id), Some(t.join("sub").as_path()));
            }
            r => panic!("expected a folder, got {r:?}"),
        }
        assert!(matches!(h.resolve("."), Ok(Resolved::Folder(_, p)) if p == t));
        std::fs::remove_dir_all(&t).ok();
    }

    #[test]
    fn resolve_relative_parent_and_absolute_paths() {
        let t = tmp("rel");
        let mut h = host_in(&t.join("sub"));
        assert_eq!(h.resolve("../a.txt"), Ok(Resolved::File(t.join("a.txt"))));
        assert_eq!(
            h.resolve("../sub/x.txt"),
            Ok(Resolved::File(t.join("sub/x.txt")))
        );
        let abs = t.join("a.txt");
        assert_eq!(
            h.resolve(abs.to_str().unwrap()),
            Ok(Resolved::File(abs.clone()))
        );
        std::fs::remove_dir_all(&t).ok();
    }

    #[test]
    fn resolve_expands_the_home_folder() {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
        let t = tmp("home");
        let mut h = host_in(&t);
        assert!(matches!(h.resolve("~"), Ok(Resolved::Folder(_, p)) if p == home));
        std::fs::remove_dir_all(&t).ok();
    }

    #[test]
    fn resolve_refuses_a_missing_parent() {
        let t = tmp("missing");
        assert!(host_in(&t).resolve("nope/x.txt").is_err());
        std::fs::remove_dir_all(&t).ok();
    }

    #[test]
    fn resolve_in_recent_documents() {
        let t = tmp("recent");
        let mut h = host_in(Path::new(reprisefs::places::RECENT));
        h.alias("a.txt".into(), t.join("a.txt").into_os_string());
        assert_eq!(h.resolve("a.txt"), Ok(Resolved::File(t.join("a.txt"))));
        assert!(h.resolve("new.txt").is_err());
        assert_eq!(
            h.resolve(&t.join("b.txt").to_string_lossy()),
            Ok(Resolved::File(t.join("b.txt"))),
            "a full path still works"
        );
        std::fs::remove_dir_all(&t).ok();
    }

    #[test]
    fn a_places_tile_gives_its_folder() {
        let mut h = Host::default();
        let id = h.place(5).expect("Computer has a folder");
        assert_eq!(h.folders.path(id), Some(Path::new("/")));
    }

    #[test]
    fn a_listed_name_that_is_not_utf8_resolves_to_its_real_bytes() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let t = tmp("latin1");
        let raw = t.join(std::ffi::OsStr::from_bytes(b"caf\xe9.txt"));
        std::fs::write(&raw, "").unwrap();
        let mut h = host_in(&t);
        let mut icons = IconCache::new(reprisekit::icontheme::IconTheme::load("test", &[]));
        let id = h.folder.unwrap();
        let mut dlg = h.icons.dialog(Fields {
            title: String::new(),
            accept_label: String::new(),
            filters: vec![],
            mode: DlgMode::Open,
            multiple: false,
        });
        h.list(&mut dlg, id, &mut icons, &|_| true);
        let shown = dlg
            .entries()
            .iter()
            .find(|e| e.name.starts_with("caf"))
            .expect("listed")
            .name
            .clone();
        assert_eq!(h.resolve(&shown), Ok(Resolved::File(raw.clone())));
        h.alias(
            "caf?.pdf".into(),
            std::ffi::OsString::from_vec(b"caf\xe9.pdf".to_vec()),
        );
        assert_eq!(
            h.resolve("caf?.pdf"),
            Ok(Resolved::File(
                t.join(std::ffi::OsStr::from_bytes(b"caf\xe9.pdf"))
            ))
        );
        std::fs::remove_dir_all(&t).ok();
    }

    #[test]
    fn a_dangling_symlink_is_taken() {
        let t = tmp("link");
        std::os::unix::fs::symlink(t.join("gone"), t.join("dangling")).unwrap();
        assert!(taken(&t.join("dangling")));
        assert!(!taken(&t.join("free.txt")));
        std::fs::remove_dir_all(&t).ok();
    }

    #[test]
    fn a_filter_keeps_folders_and_folder_mode_keeps_only_folders() {
        let t = tmp("filter");
        let mut folders = FolderTable::default();
        let id = folders.id(&t);
        let mut icons = IconCache::new(reprisekit::icontheme::IconTheme::load("test", &[]));
        let names = |f: Folder| f.entries.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
        let none = |_: &reprisefs::listing::Item| false;
        let f = listing(&mut folders, id, &mut icons, false, &none).unwrap();
        assert_eq!(
            names(f),
            vec!["sub"],
            "a filter that keeps nothing still keeps folders"
        );
        let all = |_: &reprisefs::listing::Item| true;
        let f = listing(&mut folders, id, &mut icons, true, &all).unwrap();
        assert_eq!(names(f), vec!["sub"], "folder mode lists folders only");
        std::fs::remove_dir_all(&t).ok();
    }

    #[test]
    fn the_picked_type_filters_the_listing() {
        let t = tmp("types");
        std::fs::write(t.join("bash"), "").unwrap();
        std::fs::write(t.join("b.conf"), "").unwrap();
        let mut h = host_in(&t);
        let mut icons = IconCache::new(reprisekit::icontheme::IconTheme::load("test", &[]));
        let mut dlg = h.icons.dialog(Fields {
            title: "Open".into(),
            accept_label: "Open".into(),
            filters: vec!["Text Documents (*.txt)".into(), "All Files (*.*)".into()],
            mode: DlgMode::Open,
            multiple: false,
        });
        let names = |d: &FileDlg| {
            let mut n: Vec<String> = d.entries().iter().map(|e| e.name.clone()).collect();
            n.sort();
            n
        };
        let keep = type_keep(&dlg);
        h.ensure_listed(&mut dlg, &mut icons, &keep);
        assert_eq!(names(&dlg), ["a.txt", "sub"]);
        dlg.set_filter(1);
        let id = h.folder.unwrap();
        let keep = type_keep(&dlg);
        h.list(&mut dlg, id, &mut icons, &keep);
        assert_eq!(names(&dlg), ["a.txt", "b.conf", "bash", "sub"]);
        std::fs::remove_dir_all(&t).ok();
    }

    #[test]
    fn a_typed_pattern_lists_through_it() {
        use reprisekit::controls::filedlg::Event as E;
        let t = tmp("wildcard");
        std::fs::write(t.join("bash"), "").unwrap();
        std::fs::write(t.join("sub/x.conf"), "").unwrap();
        std::fs::write(t.join("sub/y.txt"), "").unwrap();
        let mut h = host_in(&t);
        let mut icons = IconCache::new(reprisekit::icontheme::IconTheme::load("test", &[]));
        let mut dlg = h.icons.dialog(Fields {
            title: "Open".into(),
            accept_label: "Open".into(),
            filters: vec!["Text Documents (*.txt)".into(), "All Files (*.*)".into()],
            mode: DlgMode::Open,
            multiple: false,
        });
        let keep = type_keep(&dlg);
        h.ensure_listed(&mut dlg, &mut icons, &keep);
        let names = |d: &FileDlg| {
            let mut n: Vec<String> = d.entries().iter().map(|e| e.name.clone()).collect();
            n.sort();
            n
        };
        let text = TextRenderer::load(&reprisekit::theme::FontConfig::default());
        let accept = |h: &mut Host, dlg: &mut FileDlg, typed: &str| {
            dlg.set_file_name(typed);
            h.event(
                dlg,
                &key(keysym::RETURN, false, None),
                &text,
                11,
                CLIENT,
                &Metrics::default(),
            )
        };
        let Step::Out(E::NeedListing(id)) = accept(&mut h, &mut dlg, "*") else {
            panic!("a bare pattern relists the open folder");
        };
        assert_eq!(Some(id), h.folder);
        h.list(&mut dlg, id, &mut icons, &keep);
        assert_eq!(names(&dlg), ["a.txt", "bash", "sub"], "* shows every file");
        let Step::Out(E::NeedListing(id)) =
            accept(&mut h, &mut dlg, &format!("{}/sub/*.conf", t.display()))
        else {
            panic!("a folder part lists that folder");
        };
        h.list(&mut dlg, id, &mut icons, &keep);
        assert_eq!(names(&dlg), ["x.conf"]);
        assert_eq!(dlg.file_name(), "*.conf");
        match accept(&mut h, &mut dlg, "nowhere/*.txt") {
            Step::Out(E::Accept(n)) => assert_eq!(n, ["nowhere/*.txt"]),
            s => panic!("expected Accept, got {s:?}"),
        }
        assert_eq!(dlg.file_name(), "nowhere/*.txt", "the field as typed");
        assert_eq!(dlg.wildcard(), Some("*.conf"), "the filter stays");
        assert_eq!(names(&dlg), ["x.conf"], "the listing stays");
        std::fs::remove_dir_all(&t).ok();
    }

    #[test]
    fn place_icons_land_on_the_tiles_in_order() {
        let one = || Some(Canvas::new(1, 1));
        let icons = Icons {
            places: [one(), None, one(), None, one(), None],
            ..Icons::default()
        };
        let places = icons.places();
        let got: Vec<bool> = places.iter().map(|p| p.icon.is_some()).collect();
        assert_eq!(got, [true, false, true, false, true, false]);
    }

    #[test]
    fn a_listed_subfolder_carries_a_resolvable_id_and_hidden_entries_are_left_out() {
        let t = std::env::temp_dir().join(format!("repriseapp-filedlg-{}", std::process::id()));
        std::fs::create_dir_all(t.join("sub")).unwrap();
        std::fs::write(t.join("a.txt"), "").unwrap();
        std::fs::write(t.join(".hidden"), "").unwrap();
        let mut folders = FolderTable::default();
        let id = folders.id(&t);
        let mut icons = IconCache::new(reprisekit::icontheme::IconTheme::load("test", &[]));
        let f = listing(&mut folders, id, &mut icons, false, &|_| true).expect("the id resolves");
        let mut names: Vec<_> = f.entries.iter().map(|e| e.name.as_str()).collect();
        names.sort();
        assert_eq!(names, ["a.txt", "sub"]);
        let sub = f.entries.iter().find(|e| e.is_dir).expect("sub");
        assert_eq!(folders.path(sub.id), Some(t.join("sub").as_path()));
        assert_eq!(f.parent.and_then(|p| folders.path(p)), t.parent());
        assert!(listing(&mut folders, 99, &mut icons, false, &|_| true).is_none());
        std::fs::remove_dir_all(&t).ok();
    }

    #[test]
    fn the_wheel_scrolls_the_view_under_the_pointer() {
        use reprisekit::controls::filedlg::{Crumb, Folder, Hit};
        use reprisekit::controls::shellview::{self, Entry};
        use reprisekit::pointer::WHEEL_ROWS;
        let mut dlg = FileDlg::new(
            Fields {
                title: "Open".into(),
                accept_label: "Open".into(),
                filters: vec!["All Files".into()],
                mode: DlgMode::Open,
                multiple: false,
            },
            vec![],
        );
        dlg.set_listing(Folder {
            id: 1,
            parent: None,
            crumbs: vec![Crumb {
                id: 1,
                label: "Folder".into(),
                depth: 0,
                icon: None,
            }],
            entries: (0..60)
                .map(|i| Entry {
                    name: format!("f{i:02}.txt"),
                    ..Default::default()
                })
                .collect(),
        });
        let (w, h) = dlg.client();
        let r = Rect::new(0, 0, w, h);
        dlg.click(r, Some(Hit::ToolBar(3)), Mods::default());
        dlg.click(r, Some(Hit::ViewsItem(4)), Mods::default());
        let text = TextRenderer::load(&reprisekit::theme::FontConfig::default());
        let (x, y) = (0..h)
            .step_by(4)
            .flat_map(|y| (0..w).step_by(4).map(move |x| (x, y)))
            .find(|&(x, y)| {
                matches!(
                    dlg.hit(&text, 11, r, x, y),
                    Some(Hit::Shell(shellview::Hit::Entry(_)))
                )
            })
            .expect("an entry is on screen");
        let wheel = |x, y, rows| Event::Pointer(PointerInput::Scroll { x, y, rows });
        let mut host = Host::default();
        assert!(matches!(
            host.event(
                &mut dlg,
                &wheel(x, y, -WHEEL_ROWS),
                &text,
                11,
                CLIENT,
                &Metrics::default()
            ),
            Step::None
        ));
        assert!(matches!(
            host.event(
                &mut dlg,
                &wheel(x, y, WHEEL_ROWS),
                &text,
                11,
                CLIENT,
                &Metrics::default()
            ),
            Step::Redraw
        ));
        assert!(matches!(
            host.event(
                &mut dlg,
                &wheel(1, 1, WHEEL_ROWS),
                &text,
                11,
                CLIENT,
                &Metrics::default()
            ),
            Step::None
        ));
    }

    const FOLDER: u32 = 0xff20_40c0;
    const TEXT: u32 = 0xffc0_4020;
    const PICTURE: u32 = 0xff20_a040;
    const PIXELS: u32 = 0xffe0_c000;

    fn png(path: &Path, w: u32, h: u32, argb: u32) {
        let [a, r, g, b] = argb.to_be_bytes();
        image::RgbaImage::from_pixel(w, h, image::Rgba([r, g, b, a]))
            .save(path)
            .unwrap();
    }

    /// An icon theme drawing "folder", "text-x-generic" and "image-x-generic" in flat colours at
    /// 16, 32 and 48 px.
    fn flat_icons(t: &Path) -> IconCache {
        let root = t.join("icons");
        let theme = root.join("flat");
        let dirs = ["16x16/places", "32x32/places", "48x48/places"];
        let mut index = format!("[Icon Theme]\nName=Flat\nDirectories={}\n", dirs.join(","));
        for (d, px) in dirs.iter().zip([16, 32, 48]) {
            index += &format!("\n[{d}]\nSize={px}\nType=Fixed\n");
            std::fs::create_dir_all(theme.join(d)).unwrap();
            for (name, argb) in [
                ("folder", FOLDER),
                ("text-x-generic", TEXT),
                ("image-x-generic", PICTURE),
            ] {
                png(&theme.join(d).join(format!("{name}.png")), px, px, argb);
            }
        }
        std::fs::write(theme.join("index.theme"), index).unwrap();
        IconCache::new(reprisekit::icontheme::IconTheme::load("flat", &[root]))
    }

    /// `t` holds a folder, a 2 KB text file and a 40x30 picture.
    fn pictures_folder() -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("list");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("notes.txt"), [b'x'; 2048]).unwrap();
        png(&dir.join("pic.png"), 40, 30, PIXELS);
        t
    }

    fn open_dlg(h: &mut Host) -> FileDlg {
        h.icons.dialog(Fields {
            title: "Open".into(),
            accept_label: "Open".into(),
            filters: vec!["All Files".into()],
            mode: DlgMode::Open,
            multiple: false,
        })
    }

    fn entry<'a>(dlg: &'a FileDlg, name: &str) -> &'a Entry {
        dlg.entries().iter().find(|e| e.name == name).expect(name)
    }

    fn colour(icon: &Option<std::rc::Rc<Canvas>>) -> Option<(u32, u32)> {
        icon.as_ref()
            .map(|c| (c.width, c.get(c.width as i32 / 2, c.height as i32 / 2)))
    }

    /// Picks `mode` from Views the way a click does.
    fn view(h: &mut Host, dlg: &mut FileDlg, mode: Mode) {
        let i = [
            Mode::Thumbnails,
            Mode::Tiles,
            Mode::Icons,
            Mode::List,
            Mode::Details,
        ]
        .iter()
        .position(|m| *m == mode)
        .unwrap();
        let (w, ht) = dlg.client();
        let r = Rect::new(0, 0, w, ht);
        dlg.click(r, Some(Hit::ToolBar(3)), Mods::default());
        dlg.click(r, Some(Hit::ViewsItem(i)), Mods::default());
        h.ask_thumbs(dlg);
    }

    /// Takes thumbnails and icon files until some are in.
    fn settle_thumbs(h: &mut Host, dlg: &mut FileDlg) {
        for _ in 0..500 {
            if h.take_thumbs(dlg) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("nothing arrived");
    }

    fn thumbs_without_cache(h: &mut Host) {
        h.thumbs = Some(Thumbs::new(
            std::sync::Arc::new(crate::decode::Decoders::default_set()),
            None,
            crate::Waker::noop(),
        ));
    }

    #[test]
    fn a_listing_gives_entries_their_icons_at_every_size_and_tiles_lines() {
        let t = pictures_folder();
        let mut icons = flat_icons(t.path());
        let mut h = host_in(&t.path().join("list"));
        let mut dlg = open_dlg(&mut h);
        h.ensure_listed(&mut dlg, &mut icons, &|_| true);
        let sizes = |e: &Entry| [&e.icon, &e.icon32, &e.icon48].map(colour);
        let folder = entry(&dlg, "sub");
        assert_eq!(
            sizes(folder),
            [Some((16, FOLDER)), Some((32, FOLDER)), Some((48, FOLDER))]
        );
        assert!(folder.detail.is_empty(), "a folder's tile names it alone");
        let notes = entry(&dlg, "notes.txt");
        assert_eq!(
            sizes(notes),
            [Some((16, TEXT)), Some((32, TEXT)), Some((48, TEXT))]
        );
        assert_eq!(notes.detail, [notes.kind.clone(), "2 KB".into()]);
        let pic = entry(&dlg, "pic.png");
        assert_eq!(
            sizes(pic),
            [
                Some((16, PICTURE)),
                Some((32, PICTURE)),
                Some((48, PICTURE))
            ]
        );
        assert!(pic.thumb.is_none(), "no thumbnail before it is decoded");
        assert_eq!(pic.detail[0], pic.kind);
    }

    #[test]
    fn tiles_and_thumbnails_show_a_pictures_thumbnail_and_size() {
        let t = pictures_folder();
        let mut icons = flat_icons(t.path());
        let mut h = host_in(&t.path().join("list"));
        thumbs_without_cache(&mut h);
        let mut dlg = open_dlg(&mut h);
        h.ensure_listed(&mut dlg, &mut icons, &|_| true);
        assert!(!h.pictures.asked, "List decodes nothing");
        view(&mut h, &mut dlg, Mode::Tiles);
        assert!(h.pictures.asked);
        settle_thumbs(&mut h, &mut dlg);
        let pic = entry(&dlg, "pic.png");
        let thumb = pic.thumb.as_ref().expect("the picture's thumbnail");
        assert_eq!((thumb.width, thumb.height), (40, 30), "never upscaled");
        assert_eq!(thumb.get(20, 15), PIXELS);
        assert_eq!(pic.detail, ["40 x 30".to_string(), pic.kind.clone()]);
        assert!(entry(&dlg, "notes.txt").thumb.is_none());
    }

    #[test]
    fn a_new_draw_scale_replaces_the_thumbnails_made_at_the_old_one() {
        let t = pictures_folder();
        let mut icons = flat_icons(t.path());
        let mut h = host_in(&t.path().join("list"));
        thumbs_without_cache(&mut h);
        let mut dlg = open_dlg(&mut h);
        h.ensure_listed(&mut dlg, &mut icons, &|_| true);
        view(&mut h, &mut dlg, Mode::Thumbnails);
        settle_thumbs(&mut h, &mut dlg);
        let wake = crate::Waker::noop();
        h.follow_scale(&mut dlg, 1, &mut icons, &wake);
        assert!(h.thumbs.as_mut().unwrap().drain().is_empty(), "same scale");

        icons.set_scale(2);
        h.follow_scale(&mut dlg, 2, &mut icons, &wake);
        let kept = entry(&dlg, "pic.png").thumb.as_ref().expect("still shown");
        assert_eq!(kept.scale(), 1);
        settle_thumbs(&mut h, &mut dlg);
        let thumb = entry(&dlg, "pic.png").thumb.as_ref().expect("the new one");
        assert_eq!((thumb.scale(), thumb.device_size()), (2, (80, 60)));
    }

    /// Each size's canvas scale, device width and middle colour.
    fn icon_sizes(e: &Entry) -> [Option<(i32, u32, u32)>; 3] {
        [&e.icon, &e.icon32, &e.icon48].map(|c| {
            c.as_ref().map(|c| {
                let mid = c.get(c.width as i32 / 2, c.height as i32 / 2);
                (c.scale(), c.device_size().0, mid)
            })
        })
    }

    #[test]
    fn a_new_draw_scale_swaps_the_list_icons_without_listing_again() {
        const OWN: u32 = 0xff80_20a0;
        let t = pictures_folder();
        let dir = t.path().join("list");
        let own = t.path().join("own.png");
        png(&own, 96, 96, OWN);
        let directory = format!("[Desktop Entry]\nIcon={}\n", own.display());
        std::fs::write(dir.join("sub/.directory"), directory).unwrap();
        let mut icons = flat_icons(t.path());
        let mut h = host_in(&dir);
        let mut dlg = open_dlg(&mut h);
        h.ensure_listed(&mut dlg, &mut icons, &|_| true);
        let listing = h.pictures.generation;
        let wake = crate::Waker::noop();
        assert_eq!(
            icon_sizes(entry(&dlg, "notes.txt")),
            [
                Some((1, 16, TEXT)),
                Some((1, 32, TEXT)),
                Some((1, 48, TEXT))
            ]
        );

        icons.set_scale(2);
        h.follow_scale(&mut dlg, 2, &mut icons, &wake);
        assert_eq!(
            icon_sizes(entry(&dlg, "notes.txt")),
            [
                Some((2, 32, TEXT)),
                Some((2, 64, TEXT)),
                Some((2, 96, TEXT))
            ]
        );
        assert_eq!(
            icon_sizes(entry(&dlg, "sub"))[0],
            Some((1, 16, OWN)),
            "an icon file keeps its old canvas until its thread decodes it"
        );
        settle_thumbs(&mut h, &mut dlg);
        assert_eq!(
            icon_sizes(entry(&dlg, "sub")),
            [Some((2, 32, OWN)), Some((2, 64, OWN)), Some((2, 96, OWN))]
        );
        assert_eq!(
            h.pictures.generation, listing,
            "the folder is not listed again"
        );

        icons.set_scale(1);
        h.follow_scale(&mut dlg, 1, &mut icons, &wake);
        assert_eq!(
            icon_sizes(entry(&dlg, "sub")),
            [Some((1, 16, OWN)), Some((1, 32, OWN)), Some((1, 48, OWN))],
            "the cache still holds the icon file at scale 1"
        );
    }

    /// A program with a file dialog in window `DLG` that does nothing for its thumbnails.
    struct Program {
        host: Host,
        dlg: FileDlg,
    }

    const DLG: crate::WinId = "open";

    impl crate::App for Program {
        fn title(&self) -> String {
            String::new()
        }
        fn app_id(&self) -> String {
            String::new()
        }
        fn initial_size(&self) -> (i32, i32) {
            CLIENT
        }
        fn event(&mut self, _: crate::WinId, _: Event, _: &mut crate::Ctx) -> crate::Reply {
            crate::Reply::default()
        }
        fn draw(&mut self, _: crate::WinId, _: &mut Canvas, _: &crate::Ctx) {}
        fn drop_file_dialog(
            &mut self,
            win: crate::WinId,
        ) -> Option<crate::fielddrop::FileDrop<'_>> {
            (win == DLG).then_some(crate::fielddrop::FileDrop {
                dlg: &mut self.dlg,
                host: &mut self.host,
            })
        }
    }

    #[test]
    fn every_programs_file_dialog_shows_thumbnails_without_wiring() {
        let t = pictures_folder();
        let mut icons = flat_icons(t.path());
        let mut host = host_in(&t.path().join("list"));
        let dlg = open_dlg(&mut host);
        let mut app = Program { host, dlg };
        thumbs_on(&mut app, crate::MAIN, &crate::Waker::noop());
        assert!(
            app.host.thumbs.is_none(),
            "a window without a dialog starts nothing"
        );
        thumbs_on(&mut app, DLG, &crate::Waker::noop());
        assert!(
            app.host.thumbs.is_some(),
            "the dialog's window starts the worker"
        );
        thumbs_without_cache(&mut app.host);
        thumbs_on(&mut app, DLG, &crate::Waker::noop());
        let Program { host, dlg } = &mut app;
        host.ensure_listed(dlg, &mut icons, &|_| true);
        view(host, dlg, Mode::Thumbnails);
        let mut redrawn = Vec::new();
        for _ in 0..500 {
            redrawn = take_thumbs(&mut app, [crate::MAIN, DLG]);
            if !redrawn.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(redrawn, [DLG], "the dialog's window redraws");
        assert!(entry(&app.dlg, "pic.png").thumb.is_some());
        assert!(
            take_thumbs(&mut app, [DLG]).is_empty(),
            "nothing new to show"
        );
    }

    #[test]
    fn a_changed_cache_setting_restarts_the_worker_at_the_next_dialog() {
        use repriseconf::ThumbnailCache;
        let off = ThumbnailCache {
            enabled: false,
            ..ThumbnailCache::DEFAULT
        };
        let mut h = Host::default();
        h.follow_cache(off);
        thumbs_without_cache(&mut h);
        h.follow_cache(off);
        assert!(h.thumbs.is_some(), "the same setting keeps the worker");
        h.follow_cache(ThumbnailCache::DEFAULT);
        assert!(h.thumbs.is_none(), "a new setting stops it");
        h.thumbs_on(&crate::Waker::noop());
        assert!(h.thumbs.is_some());
        assert_eq!(h.cache, Some(ThumbnailCache::DEFAULT));
    }

    /// Where a colour lands in the drawn dialog: its bounding box.
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

    fn drawn(dlg: &FileDlg) -> Canvas {
        let (w, h) = dlg.client();
        let mut c = Canvas::new(w as u32, h as u32);
        let theme = reprisekit::theme::Theme::default();
        let text = TextRenderer::load(&theme.fonts);
        dlg.draw(
            &mut c,
            &text,
            &theme.paint(),
            Rect::new(0, 0, w, h),
            theme.fonts.size_px,
        );
        c
    }

    #[test]
    fn the_dialog_draws_tiles_icons_and_thumbnails_pictures() {
        let t = pictures_folder();
        let mut icons = flat_icons(t.path());
        let mut h = host_in(&t.path().join("list"));
        thumbs_without_cache(&mut h);
        let mut dlg = open_dlg(&mut h);
        h.ensure_listed(&mut dlg, &mut icons, &|_| true);
        view(&mut h, &mut dlg, Mode::Tiles);
        let tiles = drawn(&dlg);
        for argb in [FOLDER, TEXT, PICTURE] {
            let r = found(&tiles, argb).expect("a 48 px icon");
            assert_eq!((r.width, r.height), (48, 48));
        }
        let mut bare = dlg.clone();
        for e in bare.entries_mut() {
            e.detail.clear();
        }
        assert_ne!(drawn(&bare), tiles, "the kind and size lines are drawn");
        settle_thumbs(&mut h, &mut dlg);
        assert!(
            found(&drawn(&dlg), PICTURE).is_some(),
            "Tiles keeps the icon"
        );
        view(&mut h, &mut dlg, Mode::Thumbnails);
        let thumbnails = drawn(&dlg);
        let pic = found(&thumbnails, PIXELS).expect("the picture");
        assert_eq!((pic.width, pic.height), (40, 30), "at its own size");
        assert!(
            found(&thumbnails, PICTURE).is_none(),
            "the picture stands for its icon"
        );
        let r = found(&thumbnails, TEXT).expect("a 32 px icon");
        assert_eq!((r.width, r.height), (32, 32));
    }
}
