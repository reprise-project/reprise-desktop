use super::IconTheme;
use super::decode::decode;
use crate::raster::Canvas;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Cache key for one lookup; extra directories are part of it, since the same name from
/// different `IconThemePath`s is a different icon.
fn cache_key(name: &str, extra: &[PathBuf]) -> String {
    if extra.is_empty() {
        return name.to_string();
    }
    let mut key = String::from(name);
    for dir in extra {
        key.push('\0');
        key.push_str(&dir.to_string_lossy());
    }
    key
}

/// Decoded icons by `(name, px, scale)`. Misses are cached as `None`. `get_shared` costs a
/// refcount; `get` copies.
pub struct IconCache {
    theme: Arc<IconTheme>,
    cache: HashMap<(String, i32, i32), Option<Arc<Canvas>>>,
    /// The canvas scale icons come back at; see [`IconCache::set_scale`].
    scale: i32,
    /// `has` answers by name: folder scans ask per candidate name per file.
    known: RefCell<HashMap<String, bool>>,
    /// Device px `has` decodes a name at ([`IconCache::set_probe_px`]).
    probe_px: i32,
    /// The image `has` decoded for a name and its device px, until the first fetch at that size
    /// takes it.
    probed: RefCell<HashMap<String, (i32, Canvas)>>,
    /// Entries of `cache` by [`Kind`], kept for the caps.
    pub(super) counts: [usize; 3],
    pub(super) file_cap: usize,
    /// Times the icon files were dropped at their cap; see [`IconCache::file_drops`].
    file_drops: u64,
    /// Icon files [`IconCache::get_unread`] lacked, until [`IconCache::take_wanted`].
    wanted: Vec<(String, i32)>,
}

/// Names come from peers, so every set is bounded: past its cap a set is dropped whole and
/// rebuilt. Each kind is counted apart, so made-up names never evict the icons on screen and
/// theme lookups never evict the icon files a host decoded off its loop.
pub(super) const HIT_CAP: usize = 2048;
pub(super) const MISS_CAP: usize = 512;
const FILE_CAP: usize = 2048;

/// What a `cache` entry counts against: a theme icon, a miss, or an icon file decoded.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Hit = 0,
    Miss = 1,
    File = 2,
}

impl Kind {
    fn of(key: &str, canvas: &Option<Arc<Canvas>>) -> Kind {
        match canvas {
            None => Kind::Miss,
            Some(_) if key.starts_with('/') => Kind::File,
            Some(_) => Kind::Hit,
        }
    }
}

impl IconCache {
    pub fn new(theme: IconTheme) -> IconCache {
        IconCache {
            theme: Arc::new(theme),
            cache: HashMap::new(),
            scale: 1,
            known: RefCell::new(HashMap::new()),
            probe_px: PROBE_PX,
            probed: RefCell::new(HashMap::new()),
            counts: [0; 3],
            file_cap: FILE_CAP,
            file_drops: 0,
            wanted: Vec::new(),
        }
    }

    /// Looks names up in `theme` from now on. Icon files decoded, and those
    /// [`IconCache::take_wanted`] has yet to hand out, are the same in every theme and stay.
    pub fn set_theme(&mut self, theme: IconTheme) {
        self.theme = Arc::new(theme);
        self.known.get_mut().clear();
        self.probed.get_mut().clear();
        self.cache.retain(|k, _| k.0.starts_with('/'));
        self.counts = [0; 3];
        for (k, v) in &self.cache {
            self.counts[Kind::of(&k.0, v) as usize] += 1;
        }
    }

    /// [`IconCache::set_theme`] with the current theme read afresh ([`IconTheme::reload`]).
    pub fn reload(&mut self) {
        let theme = self.theme.reload();
        self.set_theme(theme);
    }

    pub fn len(&self) -> usize {
        self.cache.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }

    /// Make every later `get` return a canvas at `scale` (at least 1): a `px` icon loads at `px
    /// * scale` device pixels and keeps its logical `px`. The compositor keeps 1.
    pub fn set_scale(&mut self, scale: i32) {
        self.scale = scale.max(1);
    }

    /// Device pixels per logical pixel the program's windows draw at.
    pub fn scale(&self) -> i32 {
        self.scale
    }

    /// `has` decodes at `px` device px from now on: the size the host fetches the names it asks
    /// about at, so the first fetch at that size reads no file.
    pub fn set_probe_px(&mut self, px: i32) {
        self.probe_px = px.max(1);
    }

    pub fn probe_px(&self) -> i32 {
        self.probe_px
    }

    pub(super) fn insert(&mut self, key: (String, i32, i32), canvas: Option<Arc<Canvas>>) {
        self.make_room(Kind::of(&key.0, &canvas), 1);
        self.insert_unchecked(key, canvas);
    }

    /// Drops every entry of `kind` unless `n` more fit under its cap.
    fn make_room(&mut self, kind: Kind, n: usize) {
        let cap = match kind {
            Kind::Hit => HIT_CAP,
            Kind::Miss => MISS_CAP,
            Kind::File => self.file_cap,
        };
        if self.counts[kind as usize] + n > cap {
            self.cache.retain(|k, v| Kind::of(&k.0, v) != kind);
            self.counts[kind as usize] = 0;
            if kind == Kind::File {
                self.file_drops += 1;
            }
        }
    }

    fn insert_unchecked(&mut self, key: (String, i32, i32), canvas: Option<Arc<Canvas>>) {
        let kind = Kind::of(&key.0, &canvas);
        self.counts[kind as usize] += 1;
        if let Some(old) = self.cache.insert(key.clone(), canvas) {
            self.counts[Kind::of(&key.0, &old) as usize] -= 1;
        }
    }

    pub fn get(&mut self, name: &str, px: i32) -> Option<Canvas> {
        self.get_with_extra(name, px, &[])
    }

    /// `get` without the copy.
    pub fn get_shared(&mut self, name: &str, px: i32) -> Option<Arc<Canvas>> {
        self.shared(name, px, &[])
    }

    /// Takes icon file `path`'s image at `px`, made by [`decode_file`](super::decode_file) at this
    /// cache's scale, so `get(path, px)` answers without reading the file.
    pub fn put_file(&mut self, path: &str, px: i32, canvas: Option<Canvas>) {
        self.put_files(vec![(path.to_string(), px, canvas)]);
    }

    /// [`IconCache::put_file`] for one read's files at once: they all land, the cap dropping
    /// the files put in before them at most once, never one of these.
    pub fn put_files(&mut self, files: Vec<(String, i32, Option<Canvas>)>) {
        self.put_files_at(files, self.scale);
    }

    /// [`IconCache::put_files`] for images decoded at `scale`, which may no longer be the
    /// cache's own.
    pub fn put_files_at(&mut self, files: Vec<(String, i32, Option<Canvas>)>, scale: i32) {
        let files: Vec<_> = files
            .into_iter()
            .filter(|(path, px, _)| path.starts_with('/') && *px > 0)
            .map(|(path, px, canvas)| ((cache_key(&path, &[]), px, scale), canvas))
            .collect();
        let decoded = files.iter().filter(|(_, c)| c.is_some()).count();
        self.make_room(Kind::File, decoded);
        self.make_room(Kind::Miss, files.len() - decoded);
        for (key, canvas) in files {
            self.insert_unchecked(key, canvas.map(Arc::new));
        }
    }

    /// Bumped each time the cap drops every icon file put in: a host that saw it change since
    /// its puts reads again what [`IconCache::file_put`] no longer has.
    pub fn file_drops(&self) -> u64 {
        self.file_drops
    }

    /// Icon file `path` at `px` as [`IconCache::put_file`] left it, without reading the file:
    /// None when it was never put at this size and scale (or was dropped since).
    pub fn file_put(&self, path: &str, px: i32) -> Option<Option<Arc<Canvas>>> {
        self.cache
            .get(&(cache_key(path, &[]), px, self.scale))
            .cloned()
    }

    /// The theme `get` looks names up in, for a job loading icons with [`load_icon`] to hand
    /// to [`IconCache::put_icons`].
    pub fn theme(&self) -> Arc<IconTheme> {
        self.theme.clone()
    }

    /// Takes icons [`load_icon`] loaded from `theme` at this cache's scale, so `get` answers
    /// them without a lookup; dropped when the cache looks names up in another theme now.
    pub fn put_icons(&mut self, theme: &Arc<IconTheme>, icons: Vec<(String, i32, Option<Canvas>)>) {
        if !Arc::ptr_eq(theme, &self.theme) {
            return;
        }
        for (name, px, canvas) in icons {
            let key = (cache_key(&name, &[]), px, self.scale);
            if !self.cache.contains_key(&key) {
                self.insert(key, canvas.map(Arc::new));
            }
        }
    }

    /// [`IconCache::get_shared`] that never reads an icon file, for a host whose loop must not
    /// wait on a mount: a file not put at `px` yet is None and goes on [`IconCache::take_wanted`].
    pub fn get_unread(&mut self, name: &str, px: i32) -> Option<Option<Arc<Canvas>>> {
        if !name.starts_with('/') {
            return Some(self.get_shared(name, px));
        }
        if px <= 0 {
            return Some(None);
        }
        let put = self.file_put(name, px);
        if put.is_none() && !self.wanted.iter().any(|(f, p)| f == name && *p == px) {
            self.wanted.push((name.to_string(), px));
        }
        put
    }

    /// The icon files (path, px) [`IconCache::get_unread`] lacked since the last call, each
    /// once, for the host to [`decode_file`](super::decode_file) off its loop and
    /// [`IconCache::put_file`].
    pub fn take_wanted(&mut self) -> Vec<(String, i32)> {
        std::mem::take(&mut self.wanted)
    }

    pub fn theme_name(&self) -> String {
        self.theme.chain.first().cloned().unwrap_or_default()
    }

    /// Whether the theme chain has a file of `name` that decodes, at any size: a name whose
    /// files are all broken gives way to the next name a caller tries.
    pub fn has(&self, name: &str) -> bool {
        if let Some(&known) = self.known.borrow().get(name) {
            return known;
        }
        let px = self.probe_px;
        let art = (self.theme).find_with_extra(name, px, 1, &[], |p| decode(&p, px as u32));
        let found = art.is_some();
        let mut known = self.known.borrow_mut();
        let mut probed = self.probed.borrow_mut();
        // Peer-driven names: bounded like the cache's misses, dropped whole at the cap.
        if known.len() >= HIT_CAP {
            known.clear();
            probed.clear();
        }
        known.insert(name.to_string(), found);
        if let Some(art) = art {
            probed.insert(name.to_string(), (px, art));
        }
        found
    }

    /// `name` is an icon name or an absolute path. `extra` directories are searched first (tray
    /// `IconThemePath`) and are part of the cache key.
    pub fn get_with_extra(&mut self, name: &str, px: i32, extra: &[PathBuf]) -> Option<Canvas> {
        self.shared(name, px, extra).map(Arc::unwrap_or_clone)
    }

    fn shared(&mut self, name: &str, px: i32, extra: &[PathBuf]) -> Option<Arc<Canvas>> {
        if !loadable(name, px) {
            return None;
        }
        let key = (cache_key(name, extra), px, self.scale);
        if let Some(hit) = self.cache.get(&key) {
            return hit.clone();
        }
        // `has` looked this name up the way `load_with_extra` would at its size.
        let probe = extra.is_empty() && !name.starts_with('/') && bare_name(name) == name;
        let probed = self.probed.get_mut();
        let hit = probe
            && probed
                .get(name)
                .is_some_and(|(at, _)| *at == px * self.scale);
        let probed = hit
            .then(|| probed.remove(name))
            .flatten()
            .map(|(_, art)| art);
        let canvas = match probed {
            Some(art) => Some(art.at_scale(self.scale)),
            None => load_with_extra(&self.theme, name, px, self.scale, extra),
        };
        let canvas = canvas.map(Arc::new);
        self.insert(key, canvas.clone());
        canvas
    }
}

/// `icon`, fetched for `want` logical px with text grown by `u`, grown as the text when the theme
/// had it only smaller, up to `want`: a 16 px icon asked for at 32 px for 200% text draws at 32 px.
pub fn grown_icon(icon: Canvas, want: i32, u: crate::theme::TextUnits) -> Canvas {
    let side = u.len(icon.width as i32).min(want);
    if icon.width as i32 >= side || icon.width != icon.height {
        return icon;
    }
    icon.resample_box(side as u32, side as u32)
}

/// `name` (an icon name or an absolute path) at `px` and `scale` as [`IconCache::get`] loads it,
/// without a cache: for a job filling one through [`IconCache::put_icons`].
pub fn load_icon(theme: &IconTheme, name: &str, px: i32, scale: i32) -> Option<Canvas> {
    loadable(name, px)
        .then(|| load_with_extra(theme, name, px, scale.max(1), &[]))
        .flatten()
}

fn loadable(name: &str, px: i32) -> bool {
    // A theme icon name has no path separator; a relative path would resolve against the
    // compositor's working directory.
    !name.is_empty() && px > 0 && (name.starts_with('/') || !name.contains('/'))
}

/// `name` without the extension some `.desktop` files carry in `Icon=`; the spec says to strip it.
fn bare_name(name: &str) -> &str {
    name.strip_suffix(".png")
        .or_else(|| name.strip_suffix(".svg"))
        .or_else(|| name.strip_suffix(".xpm"))
        .unwrap_or(name)
}

/// The device px [`IconCache::has`] decodes a name at until a host sets its own: a list's small
/// icon size, which the theme usually has as a bitmap.
const PROBE_PX: i32 = 16;

fn load_with_extra(
    theme: &IconTheme,
    name: &str,
    px: i32,
    scale: i32,
    extra: &[PathBuf],
) -> Option<Canvas> {
    let dpx = px * scale;
    let canvas = if name.starts_with('/') {
        decode(Path::new(name), dpx as u32)
    } else {
        let bare = bare_name(name);
        // Theme lookup is in device pixels at scale 1: a 32 px request at 2x and a 32 px
        // icon at 1x want the same file. A file that fails to decode gives way to the next.
        let found = theme.find_with_extra(bare, dpx, 1, extra, |p| decode(&p, dpx as u32));
        if found.is_none() {
            log::debug!("no icon named \"{bare}\" decodes at {dpx} px");
        }
        found
    };
    canvas.map(|c| c.at_scale(scale))
}
