use super::icons::{ThemeOf, icons, tile_icons};
use super::*;
use crate::dialog::Outcome;
use reprisekit::controls::filedlg::Crumb;
use reprisekit::controls::shellview::{self, Entry, Mode};
use reprisekit::icontheme::{self, IconCache, ListingIcons};
use std::collections::BTreeSet;

impl FilePicker {
    /// The file system work queued since the last call, for the host to
    /// run off the loop.
    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    /// Queue a listing of `id`, or a lookup of `path`. `typed` is a
    /// typed pattern whose folder part `path` is, listed through in place
    /// of the dialog's own filter.
    pub(super) fn request(
        &mut self,
        id: Option<FolderId>,
        path: PathBuf,
        typed: Option<&str>,
    ) -> u64 {
        let mut req = Request {
            seq: 0,
            id,
            path,
            exts: self.purpose.exts(self.filter),
            pattern: typed.or(self.dlg.wildcard()).map(str::to_string),
            folder_only: typed.is_some(),
            scale: self.icons.scale,
            list: self.list_px(),
            view_file: None,
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
        if let Some(view) = r.view.filter(|_| std::mem::take(&mut self.view_awaited)) {
            self.remembered = view;
            self.dlg.restore(view);
        }
        cache.put_files(
            r.art
                .iter()
                .flat_map(|(file, art)| {
                    icontheme::listing_px(r.list)
                        .into_iter()
                        .zip(art)
                        .map(|(px, c)| (file.clone(), px * r.scale, c.clone()))
                })
                .collect(),
        );
        if self.restyling.is_some_and(|seq| seq == r.seq) {
            self.restyling = None;
            if let Found::Folder(rows) = r.found {
                self.restyle(rows, cache, r.scale, r.list);
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
                let (entries, pictures) = self.entries(rows, cache, r.scale, r.list);
                self.show(id, &r.path, entries);
                if r.scale != self.icons.scale || r.list != self.list_px() {
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

    /// The dialog draws at `scale` in `cache`'s icon theme: its icons are fetched at that scale
    /// from `cache`, and once the scale, theme or List and Details' icon size changes the
    /// listing's again off the loop;
    /// thumbnails made at another scale are dropped, since they would draw at the wrong size, and
    /// its pictures are fetched again. True when it changed.
    pub fn follow_scale(&mut self, scale: i32, cache: &mut IconCache) -> bool {
        let mut changed = false;
        let u = self.dlg.text();
        let rescaled = self.icons.scale != scale
            || self.icons.theme != ThemeOf::of(cache)
            || self.list_px() != shellview::list_icon_px(u);
        if rescaled || self.icons.text != u {
            self.icons = icons(cache, scale, u);
            let tiles = self.tiles();
            self.dlg.set_place_icons(tile_icons(&tiles, &self.icons));
            let tb = |i: usize| self.icons.toolbar.get(i).cloned().flatten();
            self.dlg.set_toolbar_icons(tb(0), tb(1), tb(2), tb(3));
            self.dlg.set_crumb_icons(vec![self.icons.folder.clone()]);
            if rescaled
                && self.listing.is_none()
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

    /// List and Details' icon size at the text size the dialog's icons were fetched at.
    fn list_px(&self) -> i32 {
        shellview::list_icon_px(self.icons.text)
    }

    /// The folder shown, relisted at `scale` and List and Details' icon size `list`: each entry
    /// still listed takes its icons. Showing another folder drops the relisting.
    fn restyle(&mut self, rows: Vec<Listed>, cache: &mut IconCache, scale: i32, list: i32) {
        let (fresh, _) = self.entries(rows, cache, scale, list);
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

    /// Thumbnails fetched for listing `generation`: each file shows it, and its Tiles lines the
    /// picture's size when known. True when one did; a listing replaced meanwhile takes none.
    pub fn thumbs(&mut self, generation: u64, done: Vec<crate::pickthumbs::Done>) -> bool {
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
            let frame = dims.unwrap_or_else(|| thumb.device_size());
            let thumb = shellview::frame_thumb(thumb, frame, self.pictures.scale).at_scale(1);
            e.thumb = Some(thumb);
            if let Some(dims) = dims {
                e.detail = shellview::image_detail(dims, &e.kind);
            }
            shown = true;
        }
        shown
    }

    /// Lists the folder shown again when it is one of `due`; true when it is.
    pub fn relist(&mut self, due: &BTreeSet<PathBuf>) -> bool {
        let Some(id) = self.dlg.folder() else {
            return false;
        };
        let Some(path) = (self.folders.path(id))
            .filter(|p| due.contains(*p))
            .map(Path::to_path_buf)
        else {
            return false;
        };
        self.listing = Some(self.request(Some(id), path, None));
        true
    }

    /// Watches the folder shown from now on with `w`, whose wake leads the host to
    /// [`FilePicker::watch_due`].
    pub fn watch_with(&mut self, mut w: reprisefs::folderwatch::Watch) {
        if let Some(dir) = self.current() {
            w.watch(dir);
            self.watched = Some(dir.to_path_buf());
        }
        self.watch = FolderWatch(Some(w));
    }

    /// Lists the folder shown again once the watch saw an entry in it change, or the folder where
    /// it went once it or one above it was renamed or moved (the nearest folder still there above
    /// it once deleted); true when that queued a request.
    pub fn watch_due(&mut self) -> bool {
        let Some(w) = &mut self.watch.0 else {
            return false;
        };
        let changed = w.take_changed();
        if let Some(gone) = w.take_gone()
            && let Some(dir) = gone.ancestors().find(|p| p.is_dir())
        {
            let id = self.folders.id(dir);
            self.list(id);
            return true;
        }
        match self.watched.clone() {
            Some(dir) if changed && self.listing.is_none() => self.relist(&BTreeSet::from([dir])),
            _ => false,
        }
    }

    /// The work on the folder shown asked for since the last call, for the host to run off the
    /// loop.
    pub fn take_ops(&mut self) -> Vec<Op> {
        std::mem::take(&mut self.ops)
    }

    /// An [`Op`] finished: the folder is read again, a new folder opening in the rename box once
    /// listed, a renamed entry selected, a refused name back in the box. True when that queued a
    /// request.
    pub fn op_done(&mut self, done: OpDone) -> bool {
        match done {
            OpDone::Named(name, rename) => self.dlg.expect(&name, rename),
            OpDone::Refused { old, typed, .. } => {
                self.dlg.resume_rename(&old, &typed);
            }
            OpDone::Nothing => {}
        }
        self.refresh()
    }

    /// Reads the folder shown again, unless its listing is still to come.
    pub(super) fn refresh(&mut self) -> bool {
        match self.current().map(Path::to_path_buf) {
            Some(dir) if self.listing.is_none() => self.relist(&BTreeSet::from([dir])),
            _ => false,
        }
    }

    /// Queues `work` on the folder shown; none in My Recent Documents, which holds links to
    /// files elsewhere.
    pub(super) fn queue(&mut self, work: Work) {
        let Some(dir) = self.current().map(Path::to_path_buf) else {
            return;
        };
        if reprisefs::places::is_recent(&dir) {
            return;
        }
        let taken = self.dlg.entries().iter().map(|e| e.name.clone()).collect();
        self.ops.push(Op { dir, taken, work });
    }

    /// Enters `id`'s folder, empty until its listing arrives.
    pub(super) fn list(&mut self, id: FolderId) {
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
    /// one resolves, and [`Pictures`]' id on every file with a thumbnail. Returns their paths too.
    pub(super) fn entries(
        &mut self,
        rows: Vec<Listed>,
        cache: &mut IconCache,
        scale: i32,
        list: i32,
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
        let mut shared = ListingIcons::device(scale).with_list(list);
        let mut pictures = Vec::new();
        let entries = rows
            .into_iter()
            .zip(names)
            .map(|(Listed { row: r, .. }, name)| {
                let it = r.item;
                let [icon, icon32, icon48] = shared.get(cache, &name);
                let id = if it.is_dir {
                    self.folders.id(&it.path)
                } else if it.thumbnailable() {
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
        if let Some(w) = &mut self.watch.0
            && self.watched.as_deref() != Some(path)
        {
            w.watch(path);
            self.watched = Some(path.to_path_buf());
        }
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
        self.mark_pinned();
    }
}
