//! The host side of `reprisekit::controls::filedlg` every program shares: places tiles, icons and
//! folder listings. Folder ids are handles in the program's `FolderTable`.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::{Event, PointerInput};
use reprisefs::listing::Item;
use reprisekit::controls::filedlg::DlgMode;
use reprisekit::controls::form;
use reprisekit::raster::Rect;
use reprisekit::selection::Mods;
use reprisekit::text::TextRenderer;
use reprisekit::textedit::keysym;

use reprisefs::folders::FolderTable;
use reprisekit::controls::filedlg::{Crumb, Fields, FileDlg, Folder, FolderId};
use reprisekit::controls::placesbar::Place;
use reprisekit::controls::shellview::Entry;
use reprisekit::icontheme::IconCache;
use reprisekit::raster::Canvas;

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
    listing_raw(folders, id, icons, folders_only, keep).map(|(f, _)| f)
}

fn listing_raw(
    folders: &mut FolderTable,
    id: FolderId,
    icons: &mut IconCache,
    folders_only: bool,
    keep: &dyn Fn(&Item) -> bool,
) -> Option<(Folder, Vec<(String, OsString)>)> {
    let path = folders.path(id)?.to_path_buf();
    let folder_icon = icons.get("folder", 16).map(Rc::new);
    let file_icon = icons.get("text-x-generic", 16).map(Rc::new);
    let globs = reprisefs::apps::mime_globs();
    let mut aliases: Vec<(String, OsString)> = Vec::new();
    let entries = reprisefs::listing::dialog_rows(&path, &globs, |i| {
        if folders_only {
            i.is_dir
        } else {
            i.is_dir || keep(i)
        }
    })
    .into_iter()
    .inspect(|r| {
        if r.path.parent() != Some(path.as_path()) {
            aliases.push((r.name.clone(), r.path.clone().into_os_string()));
        } else if let Some(raw) = r.path.file_name()
            && raw.to_str().is_none()
        {
            aliases.push((r.name.clone(), raw.to_os_string()));
        }
    })
    .map(|r| Entry {
        icon: if r.is_dir {
            folder_icon.clone()
        } else {
            file_icon.clone()
        },
        id: if r.is_dir { folders.id(&r.path) } else { 0 },
        name: r.name,
        is_dir: r.is_dir,
        link: r.is_link,
        size: r.size,
        kind: r.kind,
        modified: r.modified,
        mtime: r.mtime,
        ..Default::default()
    })
    .collect();
    let parent = reprisefs::places::dialog_parent(&path).map(|p| folders.id(p));
    let label = reprisefs::places::dialog_label(&path);
    Some((
        Folder {
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
    ))
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
    last_press: Option<u32>,
    aliases: HashMap<String, OsString>,
}

pub(crate) const DOUBLE_CLICK_MS: u32 = 500;

impl Host {
    pub fn open(&mut self, dir: &Path, fields: Fields) -> FileDlg {
        self.folder = Some(self.folders.id(dir));
        self.icons.dialog(fields)
    }

    pub fn is_double(&mut self, ev: &Event) -> bool {
        let Event::Pointer(PointerInput::Press {
            button: crate::BTN_LEFT,
            time,
            ..
        }) = ev
        else {
            return false;
        };
        let double = self
            .last_press
            .is_some_and(|t| time.wrapping_sub(t) < DOUBLE_CLICK_MS);
        self.last_press = if double { None } else { Some(*time) };
        double
    }

    pub fn event(
        &mut self,
        dlg: &mut FileDlg,
        ev: &Event,
        text: &TextRenderer,
        px: u32,
        size: (i32, i32),
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
                ..
            }) => {
                let hit = dlg.hit(text, px, r, *x, *y);
                if self.is_double(ev) {
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
        if let Some((f, aliases)) = listing_raw(&mut self.folders, id, icons, folders_only, &keep) {
            self.folder = Some(id);
            self.aliases.clear();
            self.aliases.extend(aliases);
            dlg.set_listing(f);
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
            h.event(&mut dlg, &thumb(0x113), &text, 11, CLIENT),
            Step::Out(E::NeedListing(1))
        ));
        dlg.set_listing(listing(1));
        assert!(matches!(
            h.event(&mut dlg, &thumb(0x114), &text, 11, CLIENT),
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
        host.event(&mut dlg, &key(keysym::DOWN, true, None), &text, 11, CLIENT);
        assert_eq!(dlg.file_name(), "a.txt", "Ctrl+Down moves the focus alone");
        host.event(
            &mut dlg,
            &key(keysym::SPACE, true, Some(" ")),
            &text,
            11,
            CLIENT,
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

    fn press(time: u32) -> Event {
        Event::Pointer(PointerInput::Press {
            x: 0,
            y: 0,
            button: crate::BTN_LEFT,
            time,
            mods: crate::Mods::default(),
        })
    }

    #[test]
    fn a_second_press_within_the_double_click_time_is_a_double_click() {
        let mut h = Host::default();
        assert!(!h.is_double(&press(1000)));
        assert!(h.is_double(&press(1300)), "300 ms apart");
        assert!(!h.is_double(&press(1400)), "a third press starts over");
        assert!(!h.is_double(&press(2500)), "1100 ms apart is two clicks");
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
            h.event(dlg, &key(keysym::RETURN, false, None), &text, 11, CLIENT)
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
            host.event(&mut dlg, &wheel(x, y, -WHEEL_ROWS), &text, 11, CLIENT),
            Step::None
        ));
        assert!(matches!(
            host.event(&mut dlg, &wheel(x, y, WHEEL_ROWS), &text, 11, CLIENT),
            Step::Redraw
        ));
        assert!(matches!(
            host.event(&mut dlg, &wheel(1, 1, WHEEL_ROWS), &text, 11, CLIENT),
            Step::None
        ));
    }
}
