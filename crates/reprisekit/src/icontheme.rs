//! freedesktop icon theme lookup: `index.theme` parsing, the inherits chain and the size-matching
//! rules from the Icon Theme Specification, read through each theme's current `icon-theme.cache`.

use crate::icon::premultiply;
use crate::raster::Canvas;
use crate::themecache::{self, ThemeCache};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Env var carrying the icon theme name from the compositor; clients without it load `hicolor`.
pub const ENV: &str = "REPRISE_ICONS";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirKind {
    Fixed,
    Scalable,
    Threshold,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    pub path: PathBuf,
    pub size: i32,
    pub scale: i32,
    pub kind: DirKind,
    pub min: i32,
    pub max: i32,
    pub threshold: i32,
}

impl DirEntry {
    pub fn matches(&self, size: i32, scale: i32) -> bool {
        if self.scale != scale {
            return false;
        }
        match self.kind {
            DirKind::Fixed => self.size == size,
            DirKind::Scalable => self.min <= size && size <= self.max,
            DirKind::Threshold => {
                (self.size - self.threshold) <= size && size <= (self.size + self.threshold)
            }
        }
    }

    /// Whether the biggest icon here is smaller than `size` at `scale`, so it would be scaled up.
    pub fn upscales(&self, size: i32, scale: i32) -> bool {
        let biggest = match self.kind {
            DirKind::Fixed => self.size,
            DirKind::Scalable => self.max,
            DirKind::Threshold => self.size + self.threshold,
        };
        biggest * self.scale < size * scale
    }

    /// `DirectorySizeDistance` from the spec, in pixels.
    pub fn distance(&self, size: i32, scale: i32) -> i32 {
        let want = size * scale;
        match self.kind {
            DirKind::Fixed => (self.size * self.scale - want).abs(),
            DirKind::Scalable => {
                if want < self.min * self.scale {
                    self.min * self.scale - want
                } else if want > self.max * self.scale {
                    want - self.max * self.scale
                } else {
                    0
                }
            }
            DirKind::Threshold => {
                let lo = (self.size - self.threshold) * self.scale;
                let hi = (self.size + self.threshold) * self.scale;
                if want < lo {
                    lo - want
                } else if want > hi {
                    want - hi
                } else {
                    0
                }
            }
        }
    }
}

/// Most threads [`IconTheme::warm`] reads directories on.
const WARM_THREADS: usize = 4;

const EXTENSIONS: [&str; 3] = ["png", "svg", "xpm"];
const EXTENSION_FLAGS: [u16; 3] = [themecache::PNG, themecache::SVG, themecache::XPM];

pub struct IconTheme {
    /// Theme names in lookup order: configured, its inherits, `hicolor`, then breeze and
    /// Adwaita when installed.
    pub chain: Vec<String>,
    pub dirs: Vec<DirEntry>,
    /// Range into `dirs` per theme in `chain`, so the exact-then-closest pass runs one theme at
    /// a time.
    groups: Vec<std::ops::Range<usize>>,
    roots: Vec<PathBuf>,
    /// Each theme's directory under each root that has it, and the one each entry of `dirs` is in.
    homes: Vec<Home>,
    dir_home: Vec<usize>,
    /// What each entry of `dirs`, then `roots`, holds, read on first visit so a miss costs a
    /// hash probe instead of a stat. `extra` directories are not cached.
    listings: Vec<OnceLock<Listing>>,
    /// The `dirs` indices `find_with_extra` visits per (size, scale), groups in chain order.
    orders: Mutex<HashMap<(i32, i32), DirOrder>>,
}

type DirOrder = Arc<[usize]>;

/// One theme's directory under one root.
struct Home {
    path: PathBuf,
    state: OnceLock<HomeState>,
}

enum HomeState {
    /// Not a directory when first visited: none of its size directories exist.
    Missing,
    /// No usable `icon-theme.cache`: size directories are listed, after the entries of each
    /// directory above them, so one that doesn't exist costs no probe.
    Listed(Mutex<HashMap<PathBuf, HashSet<OsString>>>),
    /// Answered from the cache, with which ancestors of a size directory are unchanged since
    /// the cache was written.
    Cached(ThemeCache, Mutex<HashMap<PathBuf, bool>>),
    /// Every size directory listed on its own.
    #[cfg(test)]
    Direct,
}

enum Listing {
    Names(HashSet<OsString>),
    /// A directory unchanged since its home's cache was written, by its index there.
    Cached(u16),
}

impl Home {
    fn state(&self) -> &HomeState {
        self.state.get_or_init(|| {
            if !self.path.is_dir() {
                HomeState::Missing
            } else if let Some(cache) = ThemeCache::open(&self.path) {
                HomeState::Cached(cache, Mutex::new(HashMap::new()))
            } else {
                HomeState::Listed(Mutex::new(HashMap::new()))
            }
        })
    }

    /// What size directory `dir` holds: listed, or from the cache when the cache still
    /// describes it.
    fn listing(&self, dir: &Path) -> Listing {
        match self.state() {
            HomeState::Missing => Listing::Names(HashSet::new()),
            HomeState::Listed(entries) => self.listed(entries, dir),
            HomeState::Cached(cache, unchanged) => self
                .cached(cache, unchanged, dir)
                .unwrap_or_else(|| Listing::Names(list_files(dir))),
            #[cfg(test)]
            HomeState::Direct => Listing::Names(list_files(dir)),
        }
    }

    /// `dir` listed, or empty without a probe when a directory above it lacks the next step;
    /// `entries` keeps each of those directories' entry names.
    fn listed(&self, entries: &Mutex<HashMap<PathBuf, HashSet<OsString>>>, dir: &Path) -> Listing {
        let rel = dir.strip_prefix(&self.path).unwrap_or(dir);
        if !rel
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
        {
            return Listing::Names(list_files(dir));
        }
        let mut entries = entries.lock().unwrap_or_else(|e| e.into_inner());
        let mut parent = PathBuf::new();
        for part in rel.iter() {
            let names = entries
                .entry(parent.clone())
                .or_insert_with(|| list_entries(&self.path.join(&parent)));
            if !names.contains(part) {
                return Listing::Names(HashSet::new());
            }
            parent.push(part);
        }
        drop(entries);
        Listing::Names(list_files(dir))
    }

    /// None when `dir` changed after `cache` was written. A directory the cache doesn't name
    /// had no icons then, if it existed: its ancestors are unchanged too.
    fn cached(
        &self,
        cache: &ThemeCache,
        unchanged: &Mutex<HashMap<PathBuf, bool>>,
        dir: &Path,
    ) -> Option<Listing> {
        let Ok(meta) = std::fs::metadata(dir) else {
            return Some(Listing::Names(HashSet::new()));
        };
        if !meta.is_dir() {
            return Some(Listing::Names(HashSet::new()));
        }
        if meta.modified().ok()? > cache.mtime {
            return None;
        }
        let rel = dir.strip_prefix(&self.path).ok()?;
        if let Some(i) = cache.dir(rel.to_str()?) {
            return Some(Listing::Cached(i));
        }
        let mut unchanged = unchanged.lock().unwrap_or_else(|e| e.into_inner());
        for parent in rel
            .ancestors()
            .skip(1)
            .filter(|p| !p.as_os_str().is_empty())
        {
            let fresh = *unchanged.entry(parent.to_path_buf()).or_insert_with(|| {
                std::fs::metadata(self.path.join(parent))
                    .and_then(|m| m.modified())
                    .is_ok_and(|t| t <= cache.mtime)
            });
            if !fresh {
                return None;
            }
        }
        Some(Listing::Names(HashSet::new()))
    }
}

fn ini_lines(text: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut section = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(s) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = s.to_string();
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            out.push((section.clone(), k.trim().to_string(), v.trim().to_string()));
        }
    }
    out
}

/// Parse a theme directory's `index.theme` into `Inherits` and directory entries; missing or
/// malformed files yield nothing.
fn parse_index(theme_dir: &Path) -> (Vec<String>, Vec<DirEntry>) {
    let Ok(text) = std::fs::read_to_string(theme_dir.join("index.theme")) else {
        return (Vec::new(), Vec::new());
    };
    let lines = ini_lines(&text);
    // The first of a repeated key wins.
    let mut values: HashMap<(&str, &str), &str> = HashMap::with_capacity(lines.len());
    for (s, k, v) in &lines {
        values.entry((s.as_str(), k.as_str())).or_insert(v.as_str());
    }
    let get = |section: &str, key: &str| values.get(&(section, key)).copied();
    let inherits: Vec<String> = get("Icon Theme", "Inherits")
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let mut names: Vec<String> = get("Icon Theme", "Directories")
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    if let Some(scaled) = get("Icon Theme", "ScaledDirectories") {
        names.extend(
            scaled
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
        );
    }
    let mut dirs = Vec::with_capacity(names.len());
    for name in names {
        let num = |key: &str| get(&name, key).and_then(|v| v.parse::<i32>().ok());
        let Some(size) = num("Size") else {
            continue;
        };
        let scale = num("Scale").unwrap_or(1).max(1);
        let kind = match get(&name, "Type") {
            Some("Fixed") => DirKind::Fixed,
            Some("Scalable") => DirKind::Scalable,
            _ => DirKind::Threshold,
        };
        let threshold = num("Threshold").unwrap_or(2);
        let (min, max) = match kind {
            DirKind::Scalable => (
                num("MinSize").unwrap_or(size),
                num("MaxSize").unwrap_or(size),
            ),
            DirKind::Threshold => (size - threshold, size + threshold),
            DirKind::Fixed => (size, size),
        };
        dirs.push(DirEntry {
            path: theme_dir.join(&name),
            size,
            scale,
            kind,
            min,
            max,
            threshold,
        });
    }
    (inherits, dirs)
}

impl IconTheme {
    /// `$XDG_DATA_HOME/icons`, `~/.icons`, each `$XDG_DATA_DIRS/icons`, then `/usr/share/pixmaps`.
    pub fn default_roots() -> Vec<PathBuf> {
        let mut roots = Vec::new();
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home.clone().map(|h| h.join(".local/share")));
        if let Some(d) = data_home {
            roots.push(d.join("icons"));
        }
        if let Some(h) = home {
            roots.push(h.join(".icons"));
        }
        // An empty XDG_DATA_DIRS counts as unset.
        let data_dirs = std::env::var("XDG_DATA_DIRS")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        for d in data_dirs.split(':').filter(|s| !s.is_empty()) {
            roots.push(PathBuf::from(d).join("icons"));
        }
        roots.push(PathBuf::from("/usr/share/pixmaps"));
        roots
    }

    /// Load `name`, its inherits (depth first, de-duplicated), then `hicolor`, breeze and
    /// Adwaita. Directories merge across roots, so a user override still sees the system copy's
    /// dirs.
    pub fn load(name: &str, roots: &[PathBuf]) -> IconTheme {
        let mut theme = IconTheme {
            chain: Vec::new(),
            dirs: Vec::new(),
            groups: Vec::new(),
            roots: roots.to_vec(),
            homes: Vec::new(),
            dir_home: Vec::new(),
            listings: Vec::new(),
            orders: Mutex::default(),
        };
        theme.walk(name);
        // Fallbacks beyond `hicolor` so standard names (`system-run`, ...) resolve with a bare
        // `hicolor`; only added when a root has their index, with their parents.
        for fallback in ["hicolor", "breeze", "Adwaita"] {
            let installed = roots
                .iter()
                .any(|r| r.join(fallback).join("index.theme").is_file());
            if fallback == "hicolor" || installed {
                theme.walk(fallback);
            }
        }
        theme.listings = (0..theme.dirs.len() + roots.len())
            .map(|_| OnceLock::new())
            .collect();
        theme
    }

    /// Append `name` and its `Inherits` chain (depth first, skipping known themes) to
    /// `chain`/`dirs`/`groups`, with a home per theme and root that has it.
    fn walk(&mut self, name: &str) {
        let IconTheme {
            chain,
            dirs,
            groups,
            roots,
            homes,
            dir_home,
            ..
        } = self;
        let mut pending: Vec<String> = vec![name.to_string()];
        while let Some(theme) = pending.pop() {
            if chain.contains(&theme) {
                continue;
            }
            chain.push(theme.clone());
            let start = dirs.len();
            let mut inherits_all: Vec<String> = Vec::new();
            // Every subdirectory any root's index names: a theme's directories count in every
            // root, with or without an `index.theme` there.
            let mut subdirs: Vec<(PathBuf, DirEntry)> = Vec::new();
            let mut seen: HashSet<PathBuf> = HashSet::new();
            // A root without the theme holds none of its directories: leaving it out keeps
            // lookups and size orders from walking them.
            let bases: Vec<PathBuf> = roots
                .iter()
                .map(|root| root.join(&theme))
                .filter(|base| base.is_dir())
                .collect();
            for base in &bases {
                let (inherits, d) = parse_index(base);
                for e in d {
                    let rel = e.path.strip_prefix(base).unwrap_or(&e.path).to_path_buf();
                    if seen.insert(rel.clone()) {
                        subdirs.push((rel, e));
                    }
                }
                inherits_all.extend(inherits);
            }
            for base in bases {
                dirs.extend(subdirs.iter().map(|(rel, e)| DirEntry {
                    path: base.join(rel),
                    ..e.clone()
                }));
                dir_home.resize(dirs.len(), homes.len());
                homes.push(Home {
                    path: base,
                    state: OnceLock::new(),
                });
            }
            groups.push(start..dirs.len());
            // Reversed so the first `Inherits` entry pops first.
            for parent in inherits_all.into_iter().rev() {
                if !chain.contains(&parent) {
                    pending.push(parent);
                }
            }
        }
    }

    /// The same theme read afresh: what an install or removal since this one read its
    /// directories changed shows. Hosts call it when a [`IconTheme::watch_dirs`] entry changes.
    pub fn reload(&self) -> IconTheme {
        IconTheme::load(&self.chain[0], &self.roots)
    }

    /// Reads every cache and checks or lists every directory now, so later lookups touch the
    /// file system only for the icon files. Hosts run it on a thread right after `load`; it
    /// spreads the directories over a few threads, so fewer are left for the first lookups on
    /// the caller's loop to read.
    pub fn warm(&self) {
        let next = AtomicUsize::new(0);
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
        std::thread::scope(|s| {
            for _ in 1..threads.min(WARM_THREADS) {
                s.spawn(|| self.warm_from(&next));
            }
            self.warm_from(&next);
        });
    }

    /// Reads the directories `warm` hands out through `next` until none is left.
    fn warm_from(&self, next: &AtomicUsize) {
        loop {
            let i = next.fetch_add(1, Ordering::Relaxed);
            let dir = match self.dirs.get(i) {
                Some(d) => &d.path,
                None => match self.roots.get(i - self.dirs.len()) {
                    Some(root) => root,
                    None => return,
                },
            };
            self.listing(i, dir);
        }
    }

    /// The existing directories whose entries lookups depend on: the roots, each theme's
    /// directory, each size directory, or the closest one above a size directory not made yet.
    pub fn watch_dirs(&self) -> Vec<PathBuf> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        let homes = self.homes.iter().map(|h| h.path.as_path());
        let sizes = self.dirs.iter().zip(&self.dir_home).filter_map(|(d, &h)| {
            let home = &self.homes[h].path;
            d.path
                .ancestors()
                .take_while(|p| p.starts_with(home) && *p != home.as_path())
                .find(|p| p.is_dir())
        });
        let tops = self.roots.iter().map(PathBuf::as_path).chain(homes);
        for dir in tops.filter(|p| p.is_dir()).chain(sizes) {
            if seen.insert(dir) {
                out.push(dir.to_path_buf());
            }
        }
        out
    }

    pub fn lookup(&self, name: &str, size: i32, scale: i32) -> Option<PathBuf> {
        self.lookup_with_extra(name, size, scale, &[])
    }

    /// `FindIconHelper` plus fallback from the spec. `extra` directories are searched flat
    /// before the chain.
    pub fn lookup_with_extra(
        &self,
        name: &str,
        size: i32,
        scale: i32,
        extra: &[PathBuf],
    ) -> Option<PathBuf> {
        self.find_with_extra(name, size, scale, extra, Some)
    }

    /// The first of `name`'s files, best first, that `take` turns into a value: the one
    /// [`IconTheme::lookup_with_extra`] picks, then the next-best sizes and the later themes.
    pub fn find_with_extra<T>(
        &self,
        name: &str,
        size: i32,
        scale: i32,
        extra: &[PathBuf],
        mut take: impl FnMut(PathBuf) -> Option<T>,
    ) -> Option<T> {
        let files = EXTENSIONS.map(|ext| OsString::from(format!("{name}.{ext}")));
        for dir in extra {
            for p in files.iter().map(|f| dir.join(f)).filter(|p| p.is_file()) {
                if let Some(t) = take(p) {
                    return Some(t);
                }
            }
        }
        for &i in self.order(size, scale).iter() {
            for p in self.files_listed(i, &self.dirs[i].path, name, &files) {
                if let Some(t) = take(p) {
                    return Some(t);
                }
            }
        }
        // Fallback: a bare file directly in a root.
        for (i, root) in self.roots.iter().enumerate() {
            for p in self.files_listed(self.dirs.len() + i, root, name, &files) {
                if let Some(t) = take(p) {
                    return Some(t);
                }
            }
        }
        None
    }

    /// One theme at a time in chain order: a themed icon, even a size mismatch, beats an
    /// inherited exact one. After the exact sizes comes the closest, but a bigger icon
    /// scaled down beats a smaller one scaled up: upscaled bitmaps blur.
    fn order(&self, size: i32, scale: i32) -> DirOrder {
        let mut orders = self.orders.lock().unwrap_or_else(|e| e.into_inner());
        let order = orders.entry((size, scale)).or_insert_with(|| {
            let mut all = Vec::with_capacity(self.dirs.len());
            for group in &self.groups {
                let (order, mut rest): (Vec<usize>, Vec<usize>) = group
                    .clone()
                    .partition(|&i| self.dirs[i].matches(size, scale));
                rest.sort_by_key(|&i| {
                    let d = &self.dirs[i];
                    (d.upscales(size, scale), d.distance(size, scale))
                });
                all.extend(order);
                all.extend(rest);
            }
            all.into()
        });
        Arc::clone(order)
    }

    /// What `dir`, entry `i` of `dirs` then `roots`, holds, read on first use.
    fn listing(&self, i: usize, dir: &Path) -> &Listing {
        self.listings[i].get_or_init(|| match self.dir_home.get(i) {
            Some(&h) => self.homes[h].listing(dir),
            None => Listing::Names(list_files(dir)),
        })
    }

    /// Those of `files`, `name` with each of [`EXTENSIONS`], directly in `dir`, through
    /// `listings[i]`, read on first use.
    fn files_listed(&self, i: usize, dir: &Path, name: &str, files: &[OsString]) -> Vec<PathBuf> {
        match self.listing(i, dir) {
            Listing::Names(names) => files
                .iter()
                .filter(|f| names.contains(*f))
                .map(|f| dir.join(f))
                .collect(),
            Listing::Cached(d) => {
                let HomeState::Cached(cache, _) = self.homes[self.dir_home[i]].state() else {
                    unreachable!("a cached listing comes from a cached home");
                };
                let flags = cache.flags(name, *d);
                files
                    .iter()
                    .zip(EXTENSION_FLAGS)
                    .filter(|&(_, f)| flags & f != 0)
                    .map(|(file, _)| dir.join(file))
                    .collect()
            }
        }
    }
}

/// File names directly in `dir` other than directories; empty if unreadable. Symlinks are not
/// followed (a theme is thousands of links); `decode` refuses non-regular files.
fn list_files(dir: &Path) -> HashSet<OsString> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter(|e| !e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| e.file_name())
                .collect()
        })
        .unwrap_or_default()
}

/// Every entry name directly in `dir`; empty if unreadable.
fn list_entries(dir: &Path) -> HashSet<OsString> {
    std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(Result::ok).map(|e| e.file_name()).collect())
        .unwrap_or_default()
}

/// Largest icon side accepted from a file or peer; bounds decode buffers at 4 MiB.
pub const MAX_ICON_DIMENSION: u32 = 1024;

/// RGBA8 straight-alpha to a premultiplied canvas; None if `rgba` is shorter than `w * h` pixels.
pub fn canvas_from_rgba(w: u32, h: u32, rgba: &[u8]) -> Option<Canvas> {
    if rgba.len() < w as usize * h as usize * 4 {
        return None;
    }
    let mut c = Canvas::new(w, h);
    for (i, px) in rgba
        .as_chunks::<4>()
        .0
        .iter()
        .take((w * h) as usize)
        .enumerate()
    {
        let straight =
            (px[3] as u32) << 24 | (px[0] as u32) << 16 | (px[1] as u32) << 8 | px[2] as u32;
        c.pixels[i] = premultiply(straight);
    }
    Some(c)
}

/// Decode a PNG of any colour type into a premultiplied canvas, refusing a side past `max_side`
/// before sizing the buffer.
pub fn png_to_canvas(bytes: &[u8], max_side: u32) -> Result<Canvas, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|e| format!("bad png header: {e}"))?;
    // Checked before the buffer is sized, so a lying header costs nothing.
    let (hw, hh) = (reader.info().width, reader.info().height);
    if hw == 0 || hh == 0 || hw > max_side || hh > max_side {
        return Err(format!("refusing a {hw}x{hh} png"));
    }
    let size = reader
        .output_buffer_size()
        .ok_or("png output buffer overflows")?;
    let mut buf = vec![0u8; size];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("png data: {e}"))?;
    let (w, h) = (info.width, info.height);
    // RGBA passes through; other layouts are widened into a copy.
    let rgba: std::borrow::Cow<[u8]> = match info.color_type {
        png::ColorType::Rgba => std::borrow::Cow::Borrowed(&buf[..info.buffer_size()]),
        png::ColorType::Rgb => buf[..info.buffer_size()]
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect::<Vec<u8>>()
            .into(),
        png::ColorType::GrayscaleAlpha => buf[..info.buffer_size()]
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect::<Vec<u8>>()
            .into(),
        png::ColorType::Grayscale => buf[..info.buffer_size()]
            .iter()
            .flat_map(|&g| [g, g, g, 255])
            .collect::<Vec<u8>>()
            .into(),
        // EXPAND turns indexed into RGB/RGBA; anything else is unsupported.
        png::ColorType::Indexed => return Err("unsupported png color type".into()),
    };
    canvas_from_rgba(w, h, &rgba).ok_or_else(|| "png data shorter than its header".into())
}

fn decode_png(path: &Path, bytes: &[u8], px: u32) -> Option<Canvas> {
    let c = match png_to_canvas(bytes, MAX_ICON_DIMENSION) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("icon {}: {e}", path.display());
            return None;
        }
    };
    let (w, h) = (c.width, c.height);
    Some(if (w, h) == (px, px) {
        c
    } else if w > px || h > px {
        shrink_to_ink(&c, px)
    } else {
        c.resample_box(px, px)
    })
}

/// Alpha below which a pixel is not ink: a faint halo must not defeat the crop.
const INK_ALPHA: u32 = 16;

/// Scale a bigger bitmap to `px` square around its ink bounding box (aspect kept, centred), so
/// a padded 128 px icon is not smaller than its neighbours.
fn shrink_to_ink(c: &Canvas, px: u32) -> Canvas {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..c.height {
        for x in 0..c.width {
            if c.get(x as i32, y as i32) >> 24 >= INK_ALPHA {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x0 >= x1 || y0 >= y1 {
        return c.resample_box(px, px);
    }
    let (bw, bh) = (x1 - x0, y1 - y0);
    let mut ink = Canvas::new(bw, bh);
    for y in 0..bh {
        for x in 0..bw {
            ink.pixels[(y * bw + x) as usize] = c.get((x0 + x) as i32, (y0 + y) as i32);
        }
    }
    // Longer side to `px`, the other in proportion (at least 1).
    let (tw, th) = if bw >= bh {
        (px, ((bh * px + bw / 2) / bw).max(1))
    } else {
        (((bw * px + bh / 2) / bh).max(1), px)
    };
    let scaled = ink.resample_box(tw, th);
    let mut out = Canvas::new(px, px);
    out.blit_over(((px - tw) / 2) as i32, ((px - th) / 2) as i32, &scaled);
    out
}

/// Render cost limits for an SVG icon: resvg has none, and a peer-named file can ask for
/// seconds of work in a few hundred bytes. Stock theme icons fit well inside.
const SVG_MAX_BYTES: usize = 1 << 20;
/// Nodes drawn, counting a mask or clip path once per element using it.
const SVG_MAX_NODES: usize = 10_000;
/// Groups drawn through an offscreen layer.
const SVG_MAX_LAYERS: usize = 1_000;
/// Filter primitives applied, each over a layer up to 4x the canvas.
const SVG_MAX_PRIMITIVES: usize = 256;
/// Path segments stroked or filled, dashes included.
const SVG_MAX_SEGMENTS: f32 = 100_000.0;

/// Why `tree` is refused, or `None` within the limits. The walk stops at the first limit passed.
fn svg_over_budget(tree: &resvg::usvg::Tree) -> Option<&'static str> {
    use resvg::usvg::{Group, Node, Path, filter::Kind};
    #[derive(Default)]
    struct Cost {
        nodes: usize,
        layers: usize,
        primitives: usize,
        segments: f32,
        over: Option<&'static str>,
    }
    impl Cost {
        fn check(&mut self) {
            if self.segments > SVG_MAX_SEGMENTS {
                self.over = Some("too many path segments");
            } else if self.nodes > SVG_MAX_NODES {
                self.over = Some("too many nodes");
            } else if self.layers > SVG_MAX_LAYERS {
                self.over = Some("too many layers");
            } else if self.primitives > SVG_MAX_PRIMITIVES {
                self.over = Some("too many filter primitives");
            }
        }
    }
    /// The segments `p` draws, plus dashes counted along the control polygon (never shorter
    /// than the curve).
    fn segments(p: &Path) -> f32 {
        let data = p.data();
        let mut n = data.verbs().len() as f32;
        if let Some(dash) = p.stroke().and_then(|s| s.dasharray()) {
            let period: f32 = dash.iter().sum();
            let length: f32 = data.points().windows(2).map(|w| w[0].distance(w[1])).sum();
            n += length / period.max(f32::MIN_POSITIVE) * dash.len() as f32;
        }
        n
    }
    fn walk(g: &Group, c: &mut Cost) {
        c.nodes += 1;
        if g.opacity().get() < 1.0
            || g.mask().is_some()
            || g.clip_path().is_some()
            || !g.filters().is_empty()
        {
            c.layers += 1;
        }
        for p in g.filters().iter().flat_map(|f| f.primitives()) {
            c.primitives += 1;
            // Per-pixel cost that grows with an attribute or a nested image; no stock theme
            // uses them.
            if matches!(
                p.kind(),
                Kind::ConvolveMatrix(_)
                    | Kind::Morphology(_)
                    | Kind::Turbulence(_)
                    | Kind::DiffuseLighting(_)
                    | Kind::SpecularLighting(_)
                    | Kind::DisplacementMap(_)
                    | Kind::Image(_)
            ) {
                c.over = Some("an unbounded filter primitive");
            }
        }
        c.check();
        for n in g.children() {
            if c.over.is_some() {
                return;
            }
            match n {
                Node::Group(child) => walk(child, c),
                other => {
                    c.nodes += 1;
                    if let Node::Path(p) = other {
                        c.segments += segments(p);
                    }
                    c.check();
                }
            }
            n.subroots(|root| {
                if c.over.is_none() {
                    walk(root, c)
                }
            });
        }
    }
    let mut cost = Cost::default();
    walk(tree.root(), &mut cost);
    cost.over
}

fn decode_svg(path: &Path, bytes: &[u8], px: u32) -> Option<Canvas> {
    use resvg::tiny_skia;
    use resvg::usvg;
    // No file references: the name comes from a peer, and usvg's default resolver would read
    // any `<image href>` path. Data URIs stay.
    let options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_string: Box::new(|_, _| None),
            ..Default::default()
        },
        ..Default::default()
    };
    if bytes.len() > SVG_MAX_BYTES {
        log::warn!("icon {}: svg larger than an icon", path.display());
        return None;
    }
    let tree = match usvg::Tree::from_data(bytes, &options) {
        Ok(t) => t,
        Err(e) => {
            log::warn!("icon {}: bad svg: {e}", path.display());
            return None;
        }
    };
    if let Some(why) = svg_over_budget(&tree) {
        log::warn!("icon {}: refusing an svg with {why}", path.display());
        return None;
    }
    let size = tree.size();
    let bound = MAX_ICON_DIMENSION as f32;
    if size.width() <= 0.0 || size.height() <= 0.0 || size.width() > bound || size.height() > bound
    {
        log::warn!(
            "icon {}: refusing a {}x{} svg",
            path.display(),
            size.width(),
            size.height()
        );
        return None;
    }
    let mut pixmap = tiny_skia::Pixmap::new(px, px)?;
    let sx = px as f32 / size.width();
    let sy = px as f32 / size.height();
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(sx, sy),
        &mut pixmap.as_mut(),
    );
    let mut c = Canvas::new(px, px);
    for (i, p) in pixmap.pixels().iter().enumerate() {
        c.pixels[i] = (p.alpha() as u32) << 24
            | (p.red() as u32) << 16
            | (p.green() as u32) << 8
            | p.blue() as u32;
    }
    Some(c)
}

/// Decode a PNG or SVG and resample to `px` square. XPM is not decoded. Sizes past
/// `MAX_ICON_DIMENSION` are refused.
pub fn decode(path: &Path, px: u32) -> Option<Canvas> {
    if px == 0 || px > MAX_ICON_DIMENSION {
        return None;
    }
    // Gate on the extension before opening: a client-supplied path like `/dev/zero` must not
    // be read.
    let ext = path.extension().and_then(|e| e.to_str());
    if !ext.is_some_and(|e| e.eq_ignore_ascii_case("png") || e.eq_ignore_ascii_case("svg")) {
        log::debug!("icon {}: unsupported file type", path.display());
        return None;
    }
    let bytes = match read_icon_file(path) {
        Ok(b) => b,
        Err(e) => {
            log::warn!("icon {}: {e}", path.display());
            return None;
        }
    };
    // The content picks the decoder: themes ship PNGs named `.svg`.
    if bytes.starts_with(PNG_SIGNATURE) {
        decode_png(path, &bytes, px)
    } else {
        decode_svg(path, &bytes, px)
    }
}

/// Icon file `path`'s image as [`IconCache::get`] loads it for `px` at `scale`, for a host that
/// reads user files off its event loop and hands the result to [`IconCache::put_file`].
pub fn decode_file(path: &Path, px: i32, scale: i32) -> Option<Canvas> {
    let dpx = u32::try_from(px.checked_mul(scale.max(1))?).ok()?;
    decode(path, dpx).map(|c| c.at_scale(scale))
}

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

const MAX_ICON_BYTES: u64 = 8 << 20;

/// Read an icon file without stalling on a peer-chosen path: non-blocking open (FIFO), regular
/// files only, read capped at `MAX_ICON_BYTES`.
fn read_icon_file(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::{Error, ErrorKind, Read};
    use std::os::unix::fs::OpenOptionsExt;
    let f = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)?;
    let meta = f.metadata()?;
    if !meta.file_type().is_file() {
        return Err(Error::new(ErrorKind::InvalidInput, "not a regular file"));
    }
    if meta.len() > MAX_ICON_BYTES {
        return Err(Error::new(ErrorKind::InvalidData, "larger than an icon"));
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    f.take(MAX_ICON_BYTES).read_to_end(&mut bytes)?;
    Ok(bytes)
}

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
    counts: [usize; 3],
    file_cap: usize,
    /// Times the icon files were dropped at their cap; see [`IconCache::file_drops`].
    file_drops: u64,
    /// Icon files [`IconCache::get_unread`] lacked, until [`IconCache::take_wanted`].
    wanted: Vec<(String, i32)>,
}

/// Names come from peers, so every set is bounded: past its cap a set is dropped whole and
/// rebuilt. Each kind is counted apart, so made-up names never evict the icons on screen and
/// theme lookups never evict the icon files a host decoded off its loop.
const HIT_CAP: usize = 2048;
const MISS_CAP: usize = 512;
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

    fn insert(&mut self, key: (String, i32, i32), canvas: Option<Arc<Canvas>>) {
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

    /// Takes icon file `path`'s image at `px`, made by [`decode_file`] at this cache's scale, so
    /// `get(path, px)` answers without reading the file.
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
    /// once, for the host to [`decode_file`] off its loop and [`IconCache::put_file`].
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

/// A file listing's icon sizes in logical px at 100% text: List and Details, Icons and
/// Thumbnails, Tiles.
pub const LISTING_PX: [i32; 3] = [16, 32, 48];

/// [`LISTING_PX`] with List and Details' icon `list` px, their icon column grown with the text.
pub fn listing_px(list: i32) -> [i32; 3] {
    [list, LISTING_PX[1], LISTING_PX[2]]
}

/// One icon name's canvases at [`ListingIcons::sizes`].
pub type ListingSizes = [Option<std::rc::Rc<Canvas>>; 3];

/// A file listing's icons by name at [`ListingIcons::sizes`]: each name is loaded once and its
/// canvases shared by every entry wearing it.
#[derive(Default)]
pub struct ListingIcons {
    by_name: HashMap<String, ListingSizes>,
    device: i32,
    /// List and Details' icon size; 0 is [`LISTING_PX`]'s.
    list: i32,
    /// The sizes theme icons are fetched at; None is all three. Icon files always hold all three.
    drawn: Option<[bool; 3]>,
}

impl ListingIcons {
    /// For a host that draws on device pixels at `scale` from a scale-1 cache: every size is
    /// fetched `scale` times as large.
    pub fn device(scale: i32) -> ListingIcons {
        ListingIcons {
            device: scale,
            ..ListingIcons::default()
        }
    }

    /// List and Details' icons fetched at `list` px ([`listing_px`]).
    pub fn with_list(self, list: i32) -> ListingIcons {
        ListingIcons { list, ..self }
    }

    /// Theme icons fetched only at the sizes a view draws, by index into
    /// [`ListingIcons::sizes`]; [`ListingIcons::draw`] adds sizes.
    pub fn drawing(self, drawn: [bool; 3]) -> ListingIcons {
        let drawn = Some(drawn);
        ListingIcons { drawn, ..self }
    }

    /// `name` from `cache` at the sizes it is held at: a theme icon at the drawn ones, an icon
    /// file at all three.
    fn fetch(&self, cache: &mut IconCache, name: &str) -> ListingSizes {
        let (k, sizes, file) = (self.device.max(1), self.sizes(), name.starts_with('/'));
        std::array::from_fn(|i| {
            (file || self.drawn.is_none_or(|d| d[i]))
                .then(|| cache.get(name, sizes[i] * k).map(std::rc::Rc::new))
                .flatten()
        })
    }

    /// A view now draws the `drawn` sizes too: every theme icon is fetched at those it lacked, and
    /// each name's old and new canvases are returned for the entries to swap at once.
    pub fn draw(
        &mut self,
        cache: &mut IconCache,
        drawn: [bool; 3],
    ) -> Vec<(ListingSizes, ListingSizes)> {
        let Some(held) = self.drawn else {
            return Vec::new();
        };
        let wider = std::array::from_fn(|i| held[i] || drawn[i]);
        if wider == held {
            return Vec::new();
        }
        self.drawn = Some(wider);
        let k = self.device.max(1);
        let sizes = self.sizes();
        let mut swaps = Vec::new();
        for (name, sizes_held) in &mut self.by_name {
            if name.starts_with('/') {
                continue;
            }
            let mut new = sizes_held.clone();
            for i in (0..3).filter(|&i| wider[i] && !held[i]) {
                new[i] = cache.get(name, sizes[i] * k).map(std::rc::Rc::new);
            }
            swaps.push((std::mem::replace(sizes_held, new.clone()), new));
        }
        swaps
    }

    /// The sizes each name is fetched at, in logical px.
    pub fn sizes(&self) -> [i32; 3] {
        match self.list {
            0 => LISTING_PX,
            list => listing_px(list),
        }
    }

    /// Fetches List and Details' icons at `list` px from now on; [`ListingIcons::refetch`]
    /// swaps the names already held. False when they already were.
    pub fn set_list(&mut self, list: i32) -> bool {
        let old = self.sizes()[0];
        self.list = list;
        old != list
    }

    pub fn get(&mut self, cache: &mut IconCache, name: &str) -> ListingSizes {
        if let Some(held) = self.by_name.get(name) {
            return held.clone();
        }
        let held = self.fetch(cache, name);
        self.by_name.insert(name.to_string(), held.clone());
        held
    }

    /// Fetches every name again from `cache`, now at another scale or list size: each name's old
    /// and new canvases. An icon file `cache` lacks at some size keeps its old ones and is
    /// returned second, for the host to decode off its loop and hand to [`ListingIcons::put`].
    pub fn refetch(
        &mut self,
        cache: &mut IconCache,
    ) -> (Vec<(ListingSizes, ListingSizes)>, Vec<String>) {
        let (k, sizes) = (self.device.max(1), self.sizes());
        let mut swaps = Vec::new();
        let mut files = Vec::new();
        let names: Vec<String> = self.by_name.keys().cloned().collect();
        for name in names {
            let new = if name.starts_with('/') {
                let put = sizes.map(|px| cache.file_put(&name, px * k));
                if put.iter().any(Option::is_none) {
                    files.push(name);
                    continue;
                }
                put.map(|c| {
                    c.flatten()
                        .map(|c| std::rc::Rc::new(Arc::unwrap_or_clone(c)))
                })
            } else {
                self.fetch(cache, &name)
            };
            let held = self.by_name.get_mut(&name).expect("a held name");
            swaps.push((std::mem::replace(held, new.clone()), new));
        }
        (swaps, files)
    }

    /// The name whose canvases include `worn`.
    pub fn name_of(&self, worn: &std::rc::Rc<Canvas>) -> Option<&str> {
        self.by_name
            .iter()
            .find(|(_, sizes)| sizes.iter().flatten().any(|c| std::rc::Rc::ptr_eq(c, worn)))
            .map(|(name, _)| name.as_str())
    }

    /// Icon file `path` as the host decoded it at [`ListingIcons::sizes`]: its old and new canvases, or
    /// None when no entry wears it.
    pub fn put(
        &mut self,
        path: &str,
        art: [Option<Canvas>; 3],
    ) -> Option<(ListingSizes, ListingSizes)> {
        let sizes = self.by_name.get_mut(path)?;
        let new = art.map(|c| c.map(std::rc::Rc::new));
        Some((std::mem::replace(sizes, new.clone()), new))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// At 200% text an icon the theme had only at 16 px for a 32 px slot draws at 32 px, a 48 px
    /// one for a 64 px slot at 64 px; at 100%, or when it already fits, it is kept.
    #[test]
    fn a_smaller_icon_grows_with_the_text_up_to_its_slot() {
        use crate::theme::TextUnits;
        let two = TextUnits::new(22, 11);
        let side = |w: u32, want: i32, u: TextUnits| grown_icon(Canvas::new(w, w), want, u).width;
        assert_eq!(side(16, 32, two), 32);
        assert_eq!(side(48, 64, two), 64);
        assert_eq!(side(64, 64, two), 64);
        assert_eq!(side(16, 32, TextUnits::ONE), 16);
    }

    /// A fresh empty directory, a different one on every call so parallel tests never share
    /// one; the caller removes it.
    fn scratch() -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("reprise-icons-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A throwaway icon root with one theme; the caller removes it.
    fn fixture(theme: &str, index: &str, files: &[&str]) -> PathBuf {
        let root = scratch();
        let dir = root.join(theme);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("index.theme"), index).unwrap();
        for f in files {
            let p = dir.join(f);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, b"").unwrap();
        }
        root
    }

    const INDEX: &str = "[Icon Theme]\nName=Test\nInherits=hicolor\nDirectories=16x16/apps,32x32/apps,scalable/apps\n\n[16x16/apps]\nSize=16\nType=Fixed\nContext=Applications\n\n[32x32/apps]\nSize=32\nType=Fixed\n\n[scalable/apps]\nSize=48\nMinSize=8\nMaxSize=512\nType=Scalable\n";

    /// A lookup lists each directory once and probes the listing after: a file that appears
    /// later is not seen until reload (the cache remembers misses anyway).
    #[test]
    fn lookups_probe_cached_listings() {
        let root = fixture("test", INDEX, &["16x16/apps/present.png"]);
        let t = IconTheme::load("test", std::slice::from_ref(&root));
        assert!(t.lookup("absent", 16, 1).is_none());
        let listed = || t.listings.iter().filter(|l| l.get().is_some()).count();
        assert_eq!(listed(), t.dirs.len() + t.roots.len(), "every dir and root");
        assert_eq!(
            t.lookup("present", 16, 1),
            Some(root.join("test/16x16/apps/present.png"))
        );
        fs::write(root.join("test/16x16/apps/late.png"), b"").unwrap();
        assert!(
            t.lookup("late", 16, 1).is_none(),
            "listed before it existed"
        );
        let extra = root.join("extra");
        fs::create_dir_all(&extra).unwrap();
        assert!(
            t.lookup_with_extra("fresh", 16, 1, std::slice::from_ref(&extra))
                .is_none()
        );
        fs::write(extra.join("fresh.png"), b"").unwrap();
        assert_eq!(
            t.lookup_with_extra("fresh", 16, 1, std::slice::from_ref(&extra)),
            Some(extra.join("fresh.png"))
        );
        fs::remove_dir_all(&root).unwrap();
    }

    /// A listing names every entry but a directory and never stats one: a symlink is taken by
    /// name even when its target is gone.
    #[test]
    fn listing_skips_directories_without_following_symlinks() {
        let root = fixture("test", INDEX, &["16x16/apps/plain.png"]);
        let dir = root.join("test/16x16/apps");
        std::os::unix::fs::symlink("plain.png", dir.join("link.png")).unwrap();
        std::os::unix::fs::symlink("gone.png", dir.join("dangling.png")).unwrap();
        fs::create_dir(dir.join("sub.png")).unwrap();
        let mut names: Vec<OsString> = list_files(&dir).into_iter().collect();
        names.sort();
        assert_eq!(names, ["dangling.png", "link.png", "plain.png"]);
        fs::remove_dir_all(&root).unwrap();
    }

    /// A root without the theme's `index.theme` still serves the directories another root's
    /// index names, ahead of later roots.
    #[test]
    fn a_root_without_the_index_serves_the_themes_directories() {
        let system = fixture("test", INDEX, &["16x16/apps/both.png"]);
        let user = scratch();
        for f in ["test/16x16/apps/mine.png", "test/16x16/apps/both.png"] {
            let p = user.join(f);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, b"").unwrap();
        }
        let t = IconTheme::load("test", &[user.clone(), system.clone()]);
        assert_eq!(
            t.lookup("mine", 16, 1),
            Some(user.join("test/16x16/apps/mine.png"))
        );
        assert_eq!(
            t.lookup("both", 16, 1),
            Some(user.join("test/16x16/apps/both.png"))
        );
        fs::remove_dir_all(&user).unwrap();
        fs::remove_dir_all(&system).unwrap();
    }

    #[test]
    fn a_repeated_key_keeps_its_first_value_and_a_repeated_directory_counts_once() {
        let index = "[Icon Theme]\nDirectories=16x16/apps,16x16/apps\n\n[16x16/apps]\nSize=16\nSize=32\nType=Fixed\n";
        let root = fixture("test", index, &[]);
        let t = IconTheme::load("test", std::slice::from_ref(&root));
        let test: Vec<_> = t.dirs[t.groups[0].clone()].iter().map(|d| d.size).collect();
        assert_eq!(test, [16]);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn parses_directories_and_inherits() {
        let root = fixture("test", INDEX, &[]);
        let t = IconTheme::load("test", std::slice::from_ref(&root));
        assert_eq!(t.chain, vec!["test".to_string(), "hicolor".to_string()]);
        assert_eq!(t.dirs.len(), 3);
        let d = &t.dirs[0];
        assert_eq!(d.size, 16);
        assert_eq!(d.scale, 1);
        assert_eq!(d.kind, DirKind::Fixed);
        let s = &t.dirs[2];
        assert_eq!(s.kind, DirKind::Scalable);
        assert_eq!((s.min, s.max), (8, 512));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn exact_size_beats_closest_size() {
        let root = fixture(
            "test",
            INDEX,
            &["16x16/apps/firefox.png", "32x32/apps/firefox.png"],
        );
        let t = IconTheme::load("test", std::slice::from_ref(&root));
        assert_eq!(
            t.lookup("firefox", 16, 1),
            Some(root.join("test/16x16/apps/firefox.png"))
        );
        assert_eq!(
            t.lookup("firefox", 32, 1),
            Some(root.join("test/32x32/apps/firefox.png"))
        );
        // Both the 16 and 32 directories are at distance 8; 32x32 wins because it scales down.
        assert_eq!(
            t.lookup("firefox", 24, 1),
            Some(root.join("test/32x32/apps/firefox.png"))
        );
        // Each size keeps its own search order once the others have been asked.
        assert_eq!(
            t.lookup("firefox", 16, 1),
            Some(root.join("test/16x16/apps/firefox.png"))
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_far_bigger_icon_beats_a_near_smaller_one() {
        let root = fixture(
            "test",
            "[Icon Theme]\nName=Test\nDirectories=16x16/apps,128x128/apps\n\n[16x16/apps]\nSize=16\nType=Fixed\n\n[128x128/apps]\nSize=128\nType=Fixed\n",
            &["16x16/apps/net.png", "128x128/apps/net.png"],
        );
        let t = IconTheme::load("test", std::slice::from_ref(&root));
        assert_eq!(
            t.lookup("net", 32, 1),
            Some(root.join("test/128x128/apps/net.png"))
        );
        // Nothing bigger: the nearest smaller one still serves.
        assert_eq!(
            t.lookup("net", 200, 1),
            Some(root.join("test/128x128/apps/net.png"))
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn scalable_dir_matches_any_size_in_range() {
        let root = fixture("test", INDEX, &["scalable/apps/gimp.svg"]);
        let t = IconTheme::load("test", std::slice::from_ref(&root));
        assert_eq!(
            t.lookup("gimp", 16, 1),
            Some(root.join("test/scalable/apps/gimp.svg"))
        );
        assert_eq!(
            t.lookup("gimp", 512, 1),
            Some(root.join("test/scalable/apps/gimp.svg"))
        );
        assert_eq!(t.lookup("nope", 16, 1), None);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn png_beats_svg_in_the_same_directory() {
        let root = fixture("test", INDEX, &["16x16/apps/a.svg", "16x16/apps/a.png"]);
        let t = IconTheme::load("test", std::slice::from_ref(&root));
        assert_eq!(
            t.lookup("a", 16, 1),
            Some(root.join("test/16x16/apps/a.png"))
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn scale_two_prefers_scaled_directory() {
        let index = "[Icon Theme]\nName=T\nDirectories=16x16/apps,16x16@2/apps\n\n[16x16/apps]\nSize=16\nType=Fixed\n\n[16x16@2/apps]\nSize=16\nScale=2\nType=Fixed\n";
        let root = fixture("test", index, &["16x16/apps/x.png", "16x16@2/apps/x.png"]);
        let t = IconTheme::load("test", std::slice::from_ref(&root));
        assert_eq!(
            t.lookup("x", 16, 2),
            Some(root.join("test/16x16@2/apps/x.png"))
        );
        assert_eq!(
            t.lookup("x", 16, 1),
            Some(root.join("test/16x16/apps/x.png"))
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn themed_larger_icon_beats_inherited_exact_size() {
        // "test" has only a 22x22 dir but inherits "hicolor" with an exact 16x16 dir: the pass
        // finishes in "test" first, so 22x22 wins.
        let root = scratch();
        let test_index = "[Icon Theme]\nName=Test\nInherits=hicolor\nDirectories=22x22/apps\n\n[22x22/apps]\nSize=22\nType=Fixed\n";
        let hicolor_index = "[Icon Theme]\nName=Hicolor\nDirectories=16x16/apps\n\n[16x16/apps]\nSize=16\nType=Fixed\n";
        for (theme, index, file) in [
            ("test", test_index, "22x22/apps/foo.png"),
            ("hicolor", hicolor_index, "16x16/apps/foo.png"),
        ] {
            let dir = root.join(theme);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("index.theme"), index).unwrap();
            let p = dir.join(file);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, b"").unwrap();
        }
        let t = IconTheme::load("test", std::slice::from_ref(&root));
        assert_eq!(
            t.lookup("foo", 16, 1),
            Some(root.join("test/22x22/apps/foo.png"))
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fallback_theme_inherits_are_followed() {
        // The configured theme is a bare "hicolor"; "Adwaita" only inherits from "Legacy", so
        // the fallback pass must walk that Inherits line.
        let root = scratch();
        let hicolor_index = "[Icon Theme]\nName=Hicolor\nDirectories=16x16/apps\n\n[16x16/apps]\nSize=16\nType=Fixed\n";
        let adwaita_index = "[Icon Theme]\nName=Adwaita\nInherits=Legacy,hicolor\nDirectories=symbolic/apps\n\n[symbolic/apps]\nSize=16\nType=Fixed\n";
        let legacy_index = "[Icon Theme]\nName=Legacy\nInherits=hicolor\nDirectories=16x16/apps\n\n[16x16/apps]\nSize=16\nType=Fixed\n";
        for (theme, index, file) in [
            ("hicolor", hicolor_index, None),
            ("Adwaita", adwaita_index, None),
            ("Legacy", legacy_index, Some("16x16/apps/system-run.png")),
        ] {
            let dir = root.join(theme);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("index.theme"), index).unwrap();
            if let Some(file) = file {
                let p = dir.join(file);
                fs::create_dir_all(p.parent().unwrap()).unwrap();
                fs::write(&p, b"").unwrap();
            }
        }
        let t = IconTheme::load("hicolor", std::slice::from_ref(&root));
        assert_eq!(
            t.chain,
            vec![
                "hicolor".to_string(),
                "Adwaita".to_string(),
                "Legacy".to_string()
            ]
        );
        assert_eq!(
            t.lookup("system-run", 16, 1),
            Some(root.join("Legacy/16x16/apps/system-run.png"))
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn falls_back_to_bare_file_in_root() {
        let root = fixture("test", INDEX, &[]);
        fs::write(root.join("bare.png"), b"").unwrap();
        let t = IconTheme::load("test", std::slice::from_ref(&root));
        assert_eq!(t.lookup("bare", 16, 1), Some(root.join("bare.png")));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn extra_roots_are_consulted_first() {
        let root = fixture("test", INDEX, &["16x16/apps/tray.png"]);
        let extra = root.join("extra");
        fs::create_dir_all(&extra).unwrap();
        fs::write(extra.join("tray.png"), b"").unwrap();
        let t = IconTheme::load("test", std::slice::from_ref(&root));
        assert_eq!(
            t.lookup_with_extra("tray", 16, 1, std::slice::from_ref(&extra)),
            Some(extra.join("tray.png"))
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn missing_theme_still_has_hicolor_in_chain() {
        let root = scratch();
        let t = IconTheme::load("does-not-exist", &[root]);
        assert_eq!(
            t.chain,
            vec!["does-not-exist".to_string(), "hicolor".to_string()]
        );
        assert!(t.dirs.is_empty());
    }

    #[test]
    fn size_matching_rules() {
        let fixed = DirEntry {
            path: PathBuf::new(),
            size: 16,
            scale: 1,
            kind: DirKind::Fixed,
            min: 16,
            max: 16,
            threshold: 2,
        };
        assert!(fixed.matches(16, 1));
        assert!(!fixed.matches(17, 1));
        assert!(!fixed.matches(16, 2));
        assert_eq!(fixed.distance(20, 1), 4);
        let thr = DirEntry {
            kind: DirKind::Threshold,
            ..fixed.clone()
        };
        assert!(thr.matches(18, 1));
        assert!(!thr.matches(19, 1));
        assert_eq!(thr.distance(19, 1), 1);
        let scalable = DirEntry {
            kind: DirKind::Scalable,
            min: 8,
            max: 64,
            ..fixed.clone()
        };
        assert!(scalable.matches(8, 1));
        assert!(scalable.matches(64, 1));
        assert!(!scalable.matches(65, 1));
        assert_eq!(scalable.distance(4, 1), 4);
        assert_eq!(scalable.distance(100, 1), 36);
    }

    fn write_png(path: &Path, w: u32, h: u32, rgba: &[u8]) {
        let file = fs::File::create(path).unwrap();
        let mut enc = png::Encoder::new(file, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().unwrap();
        writer.write_image_data(rgba).unwrap();
    }

    fn tmp(name: &str) -> PathBuf {
        let dir = scratch();
        dir.join(name)
    }

    #[test]
    fn decodes_png_and_premultiplies() {
        let p = tmp("two.png");
        // Opaque red, half-transparent green, transparent, opaque blue.
        write_png(
            &p,
            2,
            2,
            &[255, 0, 0, 255, 0, 255, 0, 128, 9, 9, 9, 0, 0, 0, 255, 255],
        );
        let c = decode(&p, 2).unwrap();
        assert_eq!((c.width, c.height), (2, 2));
        assert_eq!(c.get(0, 0), 0xFFFF_0000);
        assert_eq!(c.get(1, 0), 0x8000_8000);
        assert_eq!(c.get(0, 1), 0x0000_0000);
        assert_eq!(c.get(1, 1), 0xFF00_00FF);
        fs::remove_file(&p).unwrap();
    }

    #[test]
    fn decodes_png_and_resamples_to_requested_size() {
        let p = tmp("four.png");
        let mut rgba = Vec::new();
        for _ in 0..16 {
            rgba.extend_from_slice(&[0, 0, 255, 255]);
        }
        write_png(&p, 4, 4, &rgba);
        let c = decode(&p, 2).unwrap();
        assert_eq!((c.width, c.height), (2, 2));
        assert_eq!(c.get(1, 1), 0xFF00_00FF);
        fs::remove_file(&p).unwrap();
    }

    #[test]
    fn downscaling_crops_to_the_ink_first() {
        // 8x8 with a 4x4 opaque blue square in the middle: at 4 px the
        // square fills the whole icon instead of its centre 2x2.
        let p = tmp("ink-padded.png");
        let mut rgba = Vec::new();
        for y in 0..8 {
            for x in 0..8 {
                let ink = (2..6).contains(&x) && (2..6).contains(&y);
                rgba.extend_from_slice(if ink {
                    &[0, 0, 255, 255]
                } else {
                    &[0, 0, 0, 0]
                });
            }
        }
        write_png(&p, 8, 8, &rgba);
        let c = decode(&p, 4).unwrap();
        assert_eq!((c.width, c.height), (4, 4));
        assert!((0..4).all(|y| (0..4).all(|x| c.get(x, y) == 0xFF00_00FF)));
        fs::remove_file(&p).unwrap();

        // A wide ink (8x2 at the top of 8x8) keeps its shape: 4x1, centred.
        let p = tmp("ink-wide.png");
        let mut rgba = Vec::new();
        for y in 0..8 {
            for _ in 0..8 {
                rgba.extend_from_slice(if y < 2 {
                    &[255, 0, 0, 255]
                } else {
                    &[0, 0, 0, 0]
                });
            }
        }
        write_png(&p, 8, 8, &rgba);
        let c = decode(&p, 4).unwrap();
        let red_rows: Vec<i32> = (0..4).filter(|&y| c.get(0, y) == 0xFFFF_0000).collect();
        assert_eq!(red_rows, vec![1]);
        assert_eq!(c.get(3, 1), 0xFFFF_0000);
        fs::remove_file(&p).unwrap();
    }

    #[test]
    fn decodes_svg_at_requested_size() {
        let p = tmp("rect.svg");
        fs::write(
            &p,
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#ff0000"/></svg>"##,
        )
        .unwrap();
        let c = decode(&p, 4).unwrap();
        assert_eq!((c.width, c.height), (4, 4));
        assert_eq!(c.get(2, 2), 0xFFFF_0000);
        fs::remove_file(&p).unwrap();
    }

    /// Each renders in well under a millisecond at 48 px once its cost is cut, and takes
    /// seconds without.
    #[test]
    fn an_svg_past_the_render_budget_is_refused() {
        let svg = |body: &str| {
            let text = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48">{body}</svg>"##
            );
            resvg::usvg::Tree::from_data(text.as_bytes(), &Default::default()).unwrap()
        };
        let blur = r##"<defs><filter id="f"><feGaussianBlur stdDeviation="2"/></filter></defs>"##;
        let shape = r##"<rect width="40" height="40" fill="#f00"/>"##;
        let icon = svg(&format!(
            r##"{blur}<g filter="url(#f)">{shape}</g><g opacity=".5">{shape}</g>"##
        ));
        assert_eq!(svg_over_budget(&icon), None);

        let wide = svg(&format!(
            r##"<defs><filter id="f"><feMorphology radius="40"/></filter></defs><g filter="url(#f)">{shape}</g>"##
        ));
        assert_eq!(
            svg_over_budget(&wide),
            Some("an unbounded filter primitive")
        );
        let layers = svg(&format!(r##"<g opacity=".5">{shape}</g>"##).repeat(SVG_MAX_LAYERS + 1));
        assert_eq!(svg_over_budget(&layers), Some("too many layers"));
        let blurs = svg(&format!(
            r##"{blur}{}"##,
            format!(r##"<g filter="url(#f)">{shape}</g>"##).repeat(SVG_MAX_PRIMITIVES + 1)
        ));
        assert_eq!(svg_over_budget(&blurs), Some("too many filter primitives"));
        // 0.01 units of dash on a 1000-unit line: 100 000 dashes.
        let dashes = svg(r##"<path d="M0 0H1000" stroke="#000" stroke-dasharray="0.005 0.005"/>"##);
        assert_eq!(svg_over_budget(&dashes), Some("too many path segments"));
        // A mask is drawn once per element using it.
        let masked = svg(&format!(
            r##"<defs><mask id="m">{}</mask></defs>{}"##,
            shape.repeat(100),
            r##"<rect mask="url(#m)" width="4" height="4"/>"##.repeat(SVG_MAX_NODES / 100)
        ));
        assert_eq!(svg_over_budget(&masked), Some("too many nodes"));

        // Past the byte cap, a file is refused before it is parsed.
        let p = tmp("padded.svg");
        let head = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16"/>"##;
        fs::write(&p, format!("{head}</svg>")).unwrap();
        assert!(decode(&p, 16).is_some());
        fs::write(&p, format!("{head}{}</svg>", " ".repeat(SVG_MAX_BYTES))).unwrap();
        assert!(decode(&p, 16).is_none());
        fs::remove_file(&p).unwrap();
    }

    /// An `<image href>` naming a file on disk is not followed: the SVG renders without it.
    #[test]
    fn an_svg_does_not_read_the_files_it_names() {
        let png = tmp("secret.png");
        write_png(&png, 2, 2, &[0, 0, 255, 255].repeat(4));
        let p = tmp("leak.svg");
        fs::write(
            &p,
            format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="4" height="4"><image xlink:href="{}" width="4" height="4"/></svg>"##,
                png.display()
            ),
        )
        .unwrap();
        let c = decode(&p, 4).unwrap();
        assert!(
            c.pixels.iter().all(|&px| px == 0),
            "the file's pixels leaked in"
        );
        fs::remove_file(&p).unwrap();
        fs::remove_file(&png).unwrap();
    }

    /// Icons a job loaded from the cache's theme answer `get` without a lookup; ones loaded
    /// from a theme the cache no longer looks names up in are dropped.
    #[test]
    fn icons_put_in_from_the_caches_theme_answer_without_a_lookup() {
        let root = fixture("test", INDEX, &[]);
        let png = root.join("test/32x32/apps/app.png");
        fs::create_dir_all(png.parent().unwrap()).unwrap();
        write_png(&png, 1, 1, &[0, 0, 255, 255]);
        let mut cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
        let theme = cache.theme();
        let icon = load_icon(&theme, "app", 32, cache.scale());
        assert!(icon.is_some());
        let old = Arc::new(IconTheme::load("test", std::slice::from_ref(&root)));
        cache.put_icons(&old, vec![("app".into(), 32, None)]);
        cache.put_icons(&theme, vec![("app".into(), 32, icon.clone())]);
        fs::remove_file(&png).unwrap();
        assert_eq!(cache.get("app", 32), icon);
        fs::remove_dir_all(&root).unwrap();
    }

    /// An icon file decoded elsewhere answers `get` without the file being read again.
    #[test]
    fn a_file_put_in_answers_without_reading_the_file() {
        let root = fixture("test", INDEX, &[]);
        let pic = root.join("cover.png");
        write_png(&pic, 1, 1, &[255, 0, 0, 255]);
        let mut cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
        let name = pic.to_str().unwrap();
        let decoded = decode_file(&pic, 16, cache.scale());
        assert!(decoded.is_some());
        cache.put_file(name, 16, decoded.clone());
        fs::remove_file(&pic).unwrap();
        assert_eq!(cache.get(name, 16), decoded);
        assert_eq!(
            cache.get(name, 32),
            None,
            "a size not put in reads the file"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn get_unread_never_reads_a_file_and_wants_each_once() {
        let mut cache = IconCache::new(IconTheme::load("none", &[]));
        let file = "/srv/icons/app.png";
        assert_eq!(cache.get_unread(file, 16), None);
        assert_eq!(cache.get_unread(file, 16), None);
        assert_eq!(cache.get_unread(file, 32), None);
        assert_eq!(
            cache.take_wanted(),
            [(file.to_string(), 16), (file.to_string(), 32)]
        );
        assert!(cache.take_wanted().is_empty());
        cache.put_file(file, 16, Some(Canvas::new(16, 16)));
        cache.put_file(file, 32, None);
        assert_eq!(
            cache.get_unread(file, 16).map(|c| c.map(|c| c.width)),
            Some(Some(16))
        );
        assert_eq!(
            cache.get_unread(file, 32),
            Some(None),
            "a failed decode is not tried again"
        );
        assert_eq!(cache.get_unread("no-such-name", 16), Some(None));
        assert!(cache.take_wanted().is_empty());
    }

    /// `file_put` answers only what was put in, never reading the file: a size not put in is
    /// None though the file is there to decode.
    #[test]
    fn file_put_never_reads_the_file() {
        let root = fixture("test", INDEX, &[]);
        let pic = root.join("cover.png");
        write_png(&pic, 1, 1, &[255, 0, 0, 255]);
        let mut cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
        let name = pic.to_str().unwrap();
        let decoded = decode_file(&pic, 16, cache.scale());
        cache.put_file(name, 16, decoded.clone());
        assert_eq!(
            cache.file_put(name, 16).flatten().as_deref(),
            decoded.as_ref()
        );
        assert_eq!(cache.file_put(name, 32), None);
        cache.put_file(name, 48, None);
        assert_eq!(
            cache.file_put(name, 48),
            Some(None),
            "a failed decode is kept"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_new_theme_keeps_the_icon_files_and_the_ones_still_wanted() {
        let mut cache = IconCache::new(IconTheme::load("none", &[]));
        cache.put_file("/srv/icons/kept.png", 16, Some(Canvas::new(16, 16)));
        assert_eq!(cache.get_unread("/srv/icons/late.png", 16), None);
        cache.get_shared("missing-name", 16);
        cache.set_theme(IconTheme::load("other", &[]));
        assert_eq!(cache.theme_name(), "other");
        assert!(cache.file_put("/srv/icons/kept.png", 16).is_some());
        assert_eq!(cache.len(), 1, "the theme lookups go");
        assert_eq!(
            cache.take_wanted(),
            [("/srv/icons/late.png".to_string(), 16)]
        );
    }

    /// A file put in and then dropped at the file cap is gone from `file_put`, and `file_drops`
    /// says so; a failed decode put in is a miss and stays.
    #[test]
    fn a_file_dropped_at_the_file_cap_shows_in_file_drops() {
        let root = fixture("test", INDEX, &[]);
        let pic = root.join("cover.png");
        write_png(&pic, 1, 1, &[255, 0, 0, 255]);
        let mut cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
        cache.file_cap = 4;
        let name = pic.to_str().unwrap();
        cache.put_file(name, 16, decode_file(&pic, 16, cache.scale()));
        cache.put_file("/nowhere/broken.png", 16, None);
        let drops = cache.file_drops();
        for px in 17..20 {
            cache.put_file(name, px, decode_file(&pic, px, cache.scale()));
        }
        assert!(cache.file_put(name, 16).flatten().is_some());
        assert_eq!(cache.file_drops(), drops, "under the cap");
        cache.put_file(name, 20, decode_file(&pic, 20, cache.scale()));
        assert_eq!(cache.file_put(name, 16), None);
        assert_eq!(cache.file_drops(), drops + 1);
        assert_eq!(cache.file_put("/nowhere/broken.png", 16), Some(None));
        fs::remove_dir_all(&root).unwrap();
    }

    /// One read's files past the file cap all land, and theme lookups past the hit cap leave
    /// them in place.
    #[test]
    fn a_read_past_the_file_cap_keeps_every_file() {
        let root = fixture("test", INDEX, &[]);
        let pic = root.join("cover.png");
        write_png(&pic, 1, 1, &[255, 0, 0, 255]);
        let mut cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
        cache.file_cap = 4;
        let name = pic.to_str().unwrap();
        let read: Vec<_> = (16..22)
            .map(|px| (name.to_string(), px, decode_file(&pic, px, cache.scale())))
            .collect();
        cache.put_files(read);
        assert!(
            (16..22).all(|px| cache.file_put(name, px).flatten().is_some()),
            "every file of the read is in"
        );
        let drops = cache.file_drops();
        let icon = Arc::new(decode_file(&pic, 16, 1).unwrap());
        for i in 0..=HIT_CAP {
            cache.insert((format!("app{i}"), 16, 1), Some(icon.clone()));
        }
        assert!((16..22).all(|px| cache.file_put(name, px).flatten().is_some()));
        assert_eq!(cache.file_drops(), drops);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn decode_rejects_missing_and_garbage() {
        assert!(decode(Path::new("/nonexistent/x.png"), 16).is_none());
        let p = tmp("garbage.png");
        fs::write(&p, b"not a png").unwrap();
        assert!(decode(&p, 16).is_none());
        let s = tmp("garbage.svg");
        fs::write(&s, b"<not xml").unwrap();
        assert!(decode(&s, 16).is_none());
        fs::remove_file(&p).unwrap();
        fs::remove_file(&s).unwrap();
    }

    #[test]
    fn decode_picks_the_format_from_the_content() {
        let p = tmp("png-inside.svg");
        write_png(&p, 2, 2, &[0, 0, 255, 255].repeat(4));
        let c = decode(&p, 4).unwrap();
        assert_eq!(c.get(1, 1), 0xFF00_00FF);
        let s = tmp("svg-inside.png");
        fs::write(
            &s,
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><rect width="4" height="4" fill="#ff0000"/></svg>"##,
        )
        .unwrap();
        assert_eq!(decode(&s, 4).unwrap().get(1, 1), 0xFFFF_0000);
        fs::remove_file(&p).unwrap();
        fs::remove_file(&s).unwrap();
    }

    /// Fixed 24 and 48 directories plus a Scalable 16..512 one holding a PNG named `.svg`: a 16
    /// px request is in the scalable range, and its file is scaled down to 16.
    #[test]
    fn a_size_only_the_scalable_range_covers_loads_its_file_scaled() {
        let index = "[Icon Theme]\nName=T\nDirectories=24x24/mimetypes,48x48/mimetypes,128x128/mimetypes\n\n[24x24/mimetypes]\nSize=24\nType=Fixed\n\n[48x48/mimetypes]\nSize=48\nType=Fixed\n\n[128x128/mimetypes]\nSize=128\nMinSize=16\nMaxSize=512\nType=Scalable\n";
        let root = fixture("test", index, &[]);
        let dir = root.join("test");
        for (sub, side, rgba) in [
            ("24x24/mimetypes/pic.png", 24, [255, 0, 0, 255]),
            ("48x48/mimetypes/pic.png", 48, [0, 255, 0, 255]),
            ("128x128/mimetypes/pic.svg", 128, [0, 0, 255, 255]),
        ] {
            let p = dir.join(sub);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            write_png(&p, side, side, &rgba.repeat((side * side) as usize));
        }
        let mut cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
        for (px, want) in [(16, 0xFF00_00FF), (24, 0xFFFF_0000), (48, 0xFF00_FF00)] {
            let c = cache
                .get("pic", px)
                .unwrap_or_else(|| panic!("pic at {px}"));
            assert_eq!(
                (c.width, c.get(px / 2, px / 2)),
                (px as u32, want),
                "{px} px"
            );
        }
        fs::remove_dir_all(&root).unwrap();
    }

    /// The chain runs through the theme's own parents before `hicolor`, even when `hicolor` has
    /// the exact size.
    #[test]
    fn inherited_themes_come_before_hicolor() {
        let root = scratch();
        let test_index = "[Icon Theme]\nName=Test\nInherits=parent\nDirectories=16x16/apps\n\n[16x16/apps]\nSize=16\nType=Fixed\n";
        let parent_index = "[Icon Theme]\nName=Parent\nDirectories=32x32/apps\n\n[32x32/apps]\nSize=32\nType=Fixed\n";
        let hicolor_index = "[Icon Theme]\nName=Hicolor\nDirectories=16x16/apps\n\n[16x16/apps]\nSize=16\nType=Fixed\n";
        for (theme, index, files) in [
            ("test", test_index, &[][..]),
            ("parent", parent_index, &["32x32/apps/both.png"][..]),
            (
                "hicolor",
                hicolor_index,
                &["16x16/apps/both.png", "16x16/apps/last.png"][..],
            ),
        ] {
            let dir = root.join(theme);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("index.theme"), index).unwrap();
            for f in files {
                let p = dir.join(f);
                fs::create_dir_all(p.parent().unwrap()).unwrap();
                fs::write(&p, b"").unwrap();
            }
        }
        let t = IconTheme::load("test", std::slice::from_ref(&root));
        assert_eq!(t.chain, ["test", "parent", "hicolor"]);
        assert_eq!(
            t.lookup("both", 16, 1),
            Some(root.join("parent/32x32/apps/both.png"))
        );
        assert_eq!(
            t.lookup("last", 16, 1),
            Some(root.join("hicolor/16x16/apps/last.png"))
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn cache_resolves_by_name_and_absolute_path_and_caches_misses() {
        let root = fixture("test", INDEX, &[]);
        let icon = root.join("test/16x16/apps/cached.png");
        fs::create_dir_all(icon.parent().unwrap()).unwrap();
        write_png(&icon, 1, 1, &[0, 255, 0, 255]);
        let mut cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
        // By name through the theme.
        let c = cache.get("cached", 16).unwrap();
        assert_eq!((c.width, c.height), (16, 16));
        assert_eq!(c.get(8, 8), 0xFF00_FF00);
        // By absolute path.
        let c = cache.get(icon.to_str().unwrap(), 8).unwrap();
        assert_eq!(c.width, 8);
        // A miss is remembered: creating the file afterwards does not change the answer.
        assert!(cache.get("late", 16).is_none());
        write_png(
            &root.join("test/16x16/apps/late.png"),
            1,
            1,
            &[0, 0, 0, 255],
        );
        assert!(cache.get("late", 16).is_none());
        // A hit is remembered too: deleting the file keeps the decoded icon.
        fs::remove_file(&icon).unwrap();
        assert!(cache.get("cached", 16).is_some());
        // Different size is a different key.
        assert!(cache.get("cached", 32).is_none());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_scaled_cache_loads_the_device_size_and_keeps_the_logical_one() {
        let root = fixture("test", INDEX, &[]);
        let dir = root.join("test");
        fs::create_dir_all(dir.join("16x16/apps")).unwrap();
        fs::create_dir_all(dir.join("32x32/apps")).unwrap();
        write_png(&dir.join("16x16/apps/app.png"), 1, 1, &[255, 0, 0, 255]);
        write_png(&dir.join("32x32/apps/app.png"), 1, 1, &[0, 255, 0, 255]);
        let mut cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
        let one = cache.get("app", 16).unwrap();
        assert_eq!(
            (one.width, one.scale(), one.get(0, 0)),
            (16, 1, 0xFFFF_0000)
        );
        cache.set_scale(2);
        let two = cache.get("app", 16).unwrap();
        assert_eq!((two.width, two.height, two.scale()), (16, 16, 2));
        assert_eq!(two.device_size(), (32, 32));
        assert_eq!(two.get(0, 0), 0xFF00_FF00, "the 32 px file");
        fs::remove_dir_all(&root).unwrap();
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &b in bytes {
            crc ^= b as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    #[test]
    fn png_to_canvas_names_the_size_it_refuses() {
        let p = tmp("bound.png");
        write_png(&p, 3, 2, &[0u8; 24]);
        let bytes = fs::read(&p).unwrap();
        fs::remove_file(&p).unwrap();
        assert_eq!(png_to_canvas(&bytes, 2).unwrap_err(), "refusing a 3x2 png");
        assert_eq!(png_to_canvas(&bytes, 3).unwrap().width, 3);
        assert!(
            png_to_canvas(b"not a png", 16)
                .unwrap_err()
                .starts_with("bad png header")
        );
    }

    #[test]
    fn decode_rejects_oversized_images() {
        // A real, well-formed image past the bound.
        let wide = tmp("wide.png");
        let past = MAX_ICON_DIMENSION as usize + 1;
        write_png(&wide, past as u32, 1, &vec![255u8; past * 4]);
        assert!(decode(&wide, 16).is_none());
        fs::remove_file(&wide).unwrap();
        // A valid 1x1 file whose IHDR claims 60000x60000: rejected before a 13 GiB buffer is
        // allocated.
        let p = tmp("huge.png");
        write_png(&p, 1, 1, &[255, 0, 0, 255]);
        let mut bytes = fs::read(&p).unwrap();
        // 8 byte signature, 4 byte length, 4 byte "IHDR", then width, height.
        bytes[16..20].copy_from_slice(&60000u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&60000u32.to_be_bytes());
        let crc = crc32(&bytes[12..29]);
        bytes[29..33].copy_from_slice(&crc.to_be_bytes());
        fs::write(&p, &bytes).unwrap();
        assert!(decode(&p, 16).is_none());
        fs::remove_file(&p).unwrap();
    }

    #[test]
    fn decodes_paletted_png() {
        let p = tmp("indexed.png");
        let file = fs::File::create(&p).unwrap();
        let mut enc = png::Encoder::new(file, 1, 1);
        enc.set_color(png::ColorType::Indexed);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_palette(vec![0, 0, 255]);
        let mut writer = enc.write_header().unwrap();
        writer.write_image_data(&[0]).unwrap();
        drop(writer);
        let c = decode(&p, 1).unwrap();
        assert_eq!(c.get(0, 0), 0xFF00_00FF);
        fs::remove_file(&p).unwrap();
    }

    #[test]
    fn cache_rejects_paths_that_are_not_decodable_icons() {
        let root = fixture("test", INDEX, &[]);
        let mut cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
        // An absolute path without an icon extension is refused before opening; reading
        // /dev/zero would never end.
        assert!(cache.get("/dev/zero", 16).is_none());
        // A relative path is not a theme icon name.
        assert!(cache.get("../../dev/zero", 16).is_none());
        assert!(cache.get("apps/cached.png", 16).is_none());
        fs::remove_dir_all(&root).unwrap();
    }

    /// A FIFO or device with an icon extension is opened but not read.
    #[test]
    fn decode_refuses_files_that_are_not_regular() {
        let dir = scratch();
        let fifo = dir.join("x.png");
        let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        // SAFETY: `c` is a NUL-terminated path that outlives the call.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        assert!(decode(&fifo, 16).is_none());
        let zero = dir.join("zero.png");
        std::os::unix::fs::symlink("/dev/zero", &zero).unwrap();
        assert!(decode(&zero, 16).is_none());
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Peer-chosen names that miss are remembered up to a cap, then dropped together; hits stay.
    #[test]
    fn misses_are_bounded_and_never_evict_hits() {
        let root = fixture("test", INDEX, &["32x32/apps/app.png"]);
        write_png(
            &root.join("test/32x32/apps/app.png"),
            1,
            1,
            &[255, 0, 0, 255],
        );
        let mut cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
        assert!(cache.get("app", 32).is_some());
        for i in 0..(MISS_CAP * 3) {
            assert!(cache.get(&format!("nope-{i}"), 32).is_none());
        }
        assert!(cache.len() <= MISS_CAP + 1, "{} entries", cache.len());
        assert_eq!(cache.counts.iter().sum::<usize>(), cache.len());
        assert!(cache.get("app", 32).is_some());
        fs::remove_dir_all(&root).unwrap();
    }

    /// `has` walks the theme once per name; the answer stays if the root vanishes.
    #[test]
    fn has_remembers_its_answer_per_name() {
        let root = fixture("test", INDEX, &["32x32/apps/app.png"]);
        write_png(
            &root.join("test/32x32/apps/app.png"),
            1,
            1,
            &[255, 0, 0, 255],
        );
        let cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
        assert!(cache.has("app"));
        assert!(!cache.has("nope"));
        fs::remove_dir_all(&root).unwrap();
        assert!(cache.has("app"), "answered from memory, not the disk");
        assert!(!cache.has("nope"));
    }

    /// A file that fails to decode gives way to the name's next-best size, then to the next
    /// theme's file; the lookup still names the best file.
    #[test]
    fn a_broken_file_gives_way_to_the_next_candidate() {
        let root = fixture(
            "test",
            INDEX,
            &[
                "32x32/apps/app.png",
                "32x32/apps/gone.png",
                "16x16/apps/app.png",
            ],
        );
        let (test, hicolor) = (root.join("test"), root.join("hicolor"));
        fs::write(test.join("32x32/apps/app.png"), b"not a png").unwrap();
        fs::write(test.join("32x32/apps/gone.png"), b"not a png").unwrap();
        write_png(&test.join("16x16/apps/app.png"), 1, 1, &[255, 0, 0, 255]);
        fs::create_dir_all(hicolor.join("48x48/apps")).unwrap();
        fs::write(
            hicolor.join("index.theme"),
            "[Icon Theme]\nName=Hicolor\nDirectories=48x48/apps\n\n[48x48/apps]\nSize=48\nType=Fixed\n",
        )
        .unwrap();
        write_png(
            &hicolor.join("48x48/apps/gone.png"),
            1,
            1,
            &[0, 0, 255, 255],
        );
        let theme = IconTheme::load("test", std::slice::from_ref(&root));
        assert_eq!(
            theme.lookup("app", 32, 1),
            Some(test.join("32x32/apps/app.png"))
        );
        let mut cache = IconCache::new(theme);
        assert_eq!(
            cache.get("app", 32).map(|c| c.get(16, 16)),
            Some(0xFFFF_0000)
        );
        assert_eq!(
            cache.get("gone", 32).map(|c| c.get(16, 16)),
            Some(0xFF00_00FF)
        );
        fs::remove_dir_all(&root).unwrap();
    }

    /// A name whose every file fails to decode is not carried, so a caller moves on to its next
    /// name; one broken size among good ones still counts.
    #[test]
    fn has_answers_no_for_a_name_with_only_broken_files() {
        let root = fixture(
            "test",
            INDEX,
            &[
                "32x32/apps/broken.png",
                "32x32/apps/half.png",
                "16x16/apps/legacy.xpm",
            ],
        );
        let apps = root.join("test/32x32/apps");
        fs::write(apps.join("broken.png"), b"not a png").unwrap();
        fs::write(apps.join("half.png"), b"not a png").unwrap();
        write_png(
            &root.join("test/16x16/apps/half.png"),
            1,
            1,
            &[255, 0, 0, 255],
        );
        let cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
        assert!(!cache.has("broken"));
        assert!(!cache.has("legacy"), "xpm is not decoded");
        assert!(cache.has("half"));
        fs::remove_dir_all(&root).unwrap();
    }

    /// The first fetch at the probe size (16 device px, or the host's) takes the image `has`
    /// decoded without reading the file; other sizes still read it.
    #[test]
    fn a_fetch_at_the_probe_size_takes_what_has_decoded() {
        for (set, probe, other) in [(None, 16, 32), (Some(48), 48, 16)] {
            for scale in [1, 2] {
                let tag = format!("probe{probe}-{scale}");
                let root = fixture("test", INDEX, &["16x16/apps/app.png"]);
                write_png(
                    &root.join("test/16x16/apps/app.png"),
                    1,
                    1,
                    &[255, 0, 0, 255],
                );
                let mut cache =
                    IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
                cache.set_scale(scale);
                if let Some(px) = set {
                    cache.set_probe_px(px);
                }
                assert!(cache.has("app"));
                fs::remove_dir_all(&root).unwrap();
                let got = cache.get("app", probe / scale).expect("the probed image");
                assert_eq!(
                    (got.device_size(), got.scale(), got.get(0, 0)),
                    ((probe as u32, probe as u32), scale, 0xFFFF_0000),
                    "{tag}"
                );
                assert!(
                    cache.get("app", other).is_none(),
                    "{tag}: {other} px reads the disk"
                );
            }
        }
    }

    /// A listing drawing one size fetches a theme icon at it alone; `draw` adds the sizes a new
    /// view wants and hands the swaps, keeping the canvases already worn.
    #[test]
    fn a_listing_fetches_the_drawn_sizes_and_adds_a_new_views() {
        let root = fixture("test", INDEX, &["16x16/apps/app.png", "32x32/apps/app.png"]);
        for (f, px) in [("16x16/apps/app.png", 16), ("32x32/apps/app.png", 32)] {
            write_png(
                &root.join("test").join(f),
                px,
                px,
                &vec![255; (px * px * 4) as usize],
            );
        }
        let mut cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
        let mut icons = ListingIcons::default().drawing([false, false, true]);
        let held = icons.get(&mut cache, "app");
        let width = |s: &ListingSizes| s.clone().map(|c| c.map(|c| c.width));
        assert_eq!(width(&held), [None, None, Some(48)]);
        assert!(icons.draw(&mut cache, [false, false, true]).is_empty());

        let swaps = icons.draw(&mut cache, [true, false, false]);
        assert_eq!(swaps.len(), 1);
        let (old, new) = &swaps[0];
        assert!(std::rc::Rc::ptr_eq(
            old[2].as_ref().unwrap(),
            held[2].as_ref().unwrap()
        ));
        assert_eq!(width(new), [Some(16), None, Some(48)]);
        assert!(std::rc::Rc::ptr_eq(
            new[2].as_ref().unwrap(),
            held[2].as_ref().unwrap()
        ));
        assert_eq!(
            width(&icons.get(&mut cache, "other-app")),
            [None, None, None]
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_shared_hit_is_the_cached_canvas_itself() {
        let root = fixture("test", INDEX, &["32x32/apps/app.png"]);
        write_png(
            &root.join("test/32x32/apps/app.png"),
            1,
            1,
            &[255, 0, 0, 255],
        );
        let mut cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
        let a = cache.get_shared("app", 32).unwrap();
        let b = cache.get_shared("app", 32).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(cache.get("app", 32).unwrap().get(0, 0), 0xFFFF_0000);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn cache_keys_on_the_extra_roots_too() {
        // Two tray items publishing "app" from different IconThemePath directories must not
        // share an entry.
        let root = fixture("test", INDEX, &[]);
        let (a, b) = (root.join("a"), root.join("b"));
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        write_png(&a.join("app.png"), 1, 1, &[255, 0, 0, 255]);
        write_png(&b.join("app.png"), 1, 1, &[0, 0, 255, 255]);
        let mut cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
        let ca = cache
            .get_with_extra("app", 8, std::slice::from_ref(&a))
            .unwrap();
        let cb = cache
            .get_with_extra("app", 8, std::slice::from_ref(&b))
            .unwrap();
        assert_eq!(ca.get(4, 4), 0xFFFF_0000);
        assert_eq!(cb.get(4, 4), 0xFF00_00FF);
        fs::remove_dir_all(&root).unwrap();
    }

    /// `theme` reading every size directory from disk, as without any `icon-theme.cache`.
    fn listing_only(theme: IconTheme) -> IconTheme {
        for home in &theme.homes {
            assert!(home.state.set(HomeState::Direct).is_ok());
        }
        theme
    }

    /// Writes `theme_dir/icon-theme.cache` as `gtk-update-icon-cache` would: every directory
    /// below it holding a regular `.png`, `.svg` or `.xpm` file, symlinks followed.
    fn write_cache(theme_dir: &Path) {
        use std::collections::BTreeMap;
        let mut dirs: Vec<String> = Vec::new();
        let mut icons: BTreeMap<String, Vec<(u16, u16)>> = BTreeMap::new();
        let mut pending = vec![PathBuf::new()];
        while let Some(rel) = pending.pop() {
            let mut found: BTreeMap<String, u16> = BTreeMap::new();
            for entry in fs::read_dir(theme_dir.join(&rel)).unwrap() {
                let file = entry.unwrap().file_name().into_string().unwrap();
                let path = theme_dir.join(&rel).join(&file);
                if path.is_dir() {
                    pending.push(rel.join(&file));
                } else if !rel.as_os_str().is_empty() && path.is_file() {
                    for (ext, flag) in EXTENSIONS.iter().zip(EXTENSION_FLAGS) {
                        if let Some(stem) = file.strip_suffix(&format!(".{ext}")) {
                            *found.entry(stem.to_string()).or_default() |= flag;
                        }
                    }
                }
            }
            if !found.is_empty() {
                let d = dirs.len() as u16;
                dirs.push(rel.to_str().unwrap().to_string());
                for (name, flags) in found {
                    icons.entry(name).or_default().push((d, flags));
                }
            }
        }
        let buckets = icons.len().max(1) as u32;
        let mut out: Vec<u8> = vec![0, 1, 0, 0];
        let put = |out: &mut Vec<u8>, at: usize, v: u32| {
            out[at..at + 4].copy_from_slice(&v.to_be_bytes());
        };
        out.extend([0; 8]);
        put(&mut out, 4, 12);
        out.extend(buckets.to_be_bytes());
        out.extend(std::iter::repeat_n(0xff, 4 * buckets as usize));
        for (name, images) in &icons {
            let head = 16 + 4 * (themecache::hash(name.as_bytes()) % buckets) as usize;
            let at = out.len();
            out.extend(&out[head..head + 4].to_vec());
            out.extend([0; 8]);
            put(&mut out, head, at as u32);
            let len = out.len() as u32;
            put(&mut out, at + 4, len);
            out.extend(name.as_bytes());
            out.push(0);
            let len = out.len() as u32;
            put(&mut out, at + 8, len);
            out.extend((images.len() as u32).to_be_bytes());
            for (d, flags) in images {
                out.extend(d.to_be_bytes());
                out.extend(flags.to_be_bytes());
                out.extend([0; 4]);
            }
        }
        let list = out.len();
        put(&mut out, 8, list as u32);
        out.extend((dirs.len() as u32).to_be_bytes());
        out.extend(std::iter::repeat_n(0, 4 * dirs.len()));
        for (i, dir) in dirs.iter().enumerate() {
            let len = out.len() as u32;
            put(&mut out, list + 4 + 4 * i, len);
            out.extend(dir.as_bytes());
            out.push(0);
        }
        fs::write(theme_dir.join(themecache::FILE), out).unwrap();
    }

    fn set_mtime(path: &Path, t: std::time::SystemTime) {
        fs::File::open(path).unwrap().set_modified(t).unwrap();
    }

    /// Every directory under `dir`, `dir` included.
    fn all_dirs(dir: &Path) -> Vec<PathBuf> {
        let mut out = vec![dir.to_path_buf()];
        let mut i = 0;
        while i < out.len() {
            for e in fs::read_dir(&out[i]).unwrap().map(Result::unwrap) {
                if e.file_type().unwrap().is_dir() {
                    out.push(e.path());
                }
            }
            i += 1;
        }
        out
    }

    /// A system root with `test` (fresh cache) inheriting `base` (no cache) inheriting `hicolor`
    /// (cache stale in two directories), and a user root whose `test` has a cache older than
    /// its directory.
    fn cached_fixture() -> (PathBuf, PathBuf) {
        let top = scratch();
        let (system, user) = (top.join("system"), top.join("user"));
        let dirs = "16x16/apps,22x22/apps,24x24/apps,32x32/apps,48x48/apps,64x64/apps,scalable/apps,16x16@2/apps";
        let sections = "[16x16/apps]\nSize=16\nType=Fixed\n\n[22x22/apps]\nSize=22\nType=Fixed\n\n[24x24/apps]\nSize=24\n\n[32x32/apps]\nSize=32\nType=Fixed\n\n[48x48/apps]\nSize=48\nType=Fixed\n\n[64x64/apps]\nSize=64\nType=Fixed\n\n[scalable/apps]\nSize=48\nMinSize=8\nMaxSize=512\nType=Scalable\n\n[16x16@2/apps]\nSize=16\nScale=2\nType=Fixed\n";
        let themes: [(&Path, &str, &str, &[&str]); 4] = [
            (
                &system,
                "test",
                "base",
                &[
                    "16x16/apps/a.png",
                    "16x16/apps/b.svg",
                    "16x16/apps/both.png",
                    "16x16/apps/both.svg",
                    "24x24/apps/t.xpm",
                    "24x24/apps/t.png",
                    "32x32/apps/a.png",
                    "48x48/apps/x.svgz",
                    "scalable/apps/s.svg",
                    "scalable/apps/a.svg",
                    "16x16@2/apps/a.png",
                    "16x16@2/apps/b.png",
                ],
            ),
            (&user, "test", "", &["16x16/apps/u.png", "32x32/apps/a.svg"]),
            (
                &system,
                "base",
                "hicolor",
                &[
                    "16x16/apps/b.png",
                    "16x16/apps/base.png",
                    "32x32/apps/base.svg",
                    "scalable/apps/u.svg",
                ],
            ),
            (
                &system,
                "hicolor",
                "",
                &[
                    "16x16/apps/h.png",
                    "16x16/apps/gone.png",
                    "48x48/apps/h.png",
                    "scalable/apps/h.svg",
                    "scalable/apps/t.svg",
                ],
            ),
        ];
        for (root, theme, inherits, files) in themes {
            let dir = root.join(theme);
            fs::create_dir_all(&dir).unwrap();
            if root == system {
                let index = format!(
                    "[Icon Theme]\nName={theme}\nInherits={inherits}\nDirectories={dirs}\n\n{sections}"
                );
                fs::write(dir.join("index.theme"), index).unwrap();
            }
            for f in files {
                let p = dir.join(f);
                fs::create_dir_all(p.parent().unwrap()).unwrap();
                fs::write(&p, b"").unwrap();
            }
        }
        std::os::unix::fs::symlink("gone.png", system.join("test/16x16/apps/dangling.png"))
            .unwrap();
        fs::write(system.join("bare.png"), b"").unwrap();
        for theme in [
            system.join("test"),
            system.join("hicolor"),
            user.join("test"),
        ] {
            write_cache(&theme);
        }
        let written = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(2_000_000);
        let before = written - std::time::Duration::from_secs(1000);
        for dir in all_dirs(&top) {
            set_mtime(&dir, before);
        }
        for theme in [
            system.join("test"),
            system.join("hicolor"),
            user.join("test"),
        ] {
            set_mtime(&theme.join(themecache::FILE), written);
        }
        // Changed after the caches were written.
        fs::remove_file(system.join("hicolor/16x16/apps/gone.png")).unwrap();
        fs::write(system.join("hicolor/48x48/apps/late.png"), b"").unwrap();
        fs::create_dir_all(system.join("test/22x22/apps")).unwrap();
        fs::write(system.join("test/22x22/apps/new.png"), b"").unwrap();
        // Backdated like a copy keeping times: only its new parent shows the change.
        set_mtime(&system.join("test/22x22/apps"), before);
        set_mtime(&system.join("test"), before);
        set_mtime(
            &user.join("test"),
            written + std::time::Duration::from_secs(1),
        );
        (system, user)
    }

    /// A file a lookup can use: `decode` refuses anything but a regular file.
    fn regular(p: PathBuf) -> Option<PathBuf> {
        p.is_file().then_some(p)
    }

    const NAMES: [&str; 17] = [
        "a",
        "b",
        "both",
        "t",
        "s",
        "u",
        "x",
        "base",
        "h",
        "gone",
        "late",
        "new",
        "dangling",
        "bare",
        "index",
        "icon-theme",
        "nothing",
    ];

    #[test]
    fn a_theme_cache_picks_the_files_the_listings_pick() {
        let (system, user) = cached_fixture();
        let roots = [user.clone(), system.clone()];
        let cached = IconTheme::load("test", &roots);
        let listed = listing_only(IconTheme::load("test", &roots));
        for name in NAMES {
            for size in [8, 16, 20, 22, 24, 32, 40, 48, 64, 96, 256, 600] {
                for scale in [1, 2] {
                    let want = listed.find_with_extra(name, size, scale, &[], regular);
                    let got = cached.find_with_extra(name, size, scale, &[], regular);
                    assert_eq!(got, want, "{name} at {size}@{scale}");
                    if name != "dangling" {
                        assert_eq!(
                            cached.lookup(name, size, scale),
                            listed.lookup(name, size, scale),
                            "{name} at {size}@{scale}"
                        );
                    }
                }
            }
        }
        assert_eq!(
            cached.lookup("late", 48, 1),
            Some(system.join("hicolor/48x48/apps/late.png"))
        );
        assert_eq!(
            cached.lookup("new", 22, 1),
            Some(system.join("test/22x22/apps/new.png"))
        );
        assert_eq!(
            cached.lookup("u", 16, 1),
            Some(user.join("test/16x16/apps/u.png"))
        );
        // The listing names a dangling link that never decodes; the cache skips it.
        assert_eq!(
            listed.lookup("dangling", 16, 1),
            Some(system.join("test/16x16/apps/dangling.png"))
        );
        assert_eq!(cached.lookup("dangling", 16, 1), None);
        fs::remove_dir_all(system.parent().unwrap()).unwrap();
    }

    #[test]
    fn an_icon_installed_or_removed_after_a_lookup_shows_once_reloaded() {
        let (system, user) = cached_fixture();
        let mut icons = IconCache::new(IconTheme::load("test", &[user.clone(), system.clone()]));
        let apps = system.join("test/16x16/apps");
        let t = icons.theme();
        assert_eq!(t.lookup("fresh", 16, 1), None);
        assert_eq!(
            t.lookup("t", 16, 1),
            Some(system.join("test/24x24/apps/t.png"))
        );
        assert_eq!(t.lookup("b", 16, 1), Some(apps.join("b.svg")));
        assert!(t.watch_dirs().contains(&apps));
        fs::write(apps.join("fresh.png"), b"").unwrap();
        fs::write(apps.join("t.png"), b"").unwrap();
        fs::remove_file(apps.join("b.svg")).unwrap();
        assert_eq!(t.lookup("fresh", 16, 1), None, "read once until reloaded");
        icons.reload();
        let t = icons.theme();
        assert_eq!(t.lookup("fresh", 16, 1), Some(apps.join("fresh.png")));
        assert_eq!(t.lookup("t", 16, 1), Some(apps.join("t.png")));
        assert_eq!(
            t.lookup("b", 16, 1),
            Some(system.join("test/16x16@2/apps/b.png"))
        );
        fs::remove_file(apps.join("fresh.png")).unwrap();
        assert_eq!(t.reload().lookup("fresh", 16, 1), None);
        fs::remove_dir_all(system.parent().unwrap()).unwrap();
    }

    #[test]
    fn watched_directories_are_the_roots_homes_and_each_size_directory_or_its_parent() {
        let (system, user) = cached_fixture();
        let t = IconTheme::load("test", &[user.clone(), system.clone()]);
        let w = t.watch_dirs();
        for dir in [
            user.clone(),
            system.clone(),
            user.join("test"),
            system.join("hicolor"),
            system.join("test/16x16/apps"),
            system.join("test/22x22/apps"),
            system.join("hicolor/48x48/apps"),
        ] {
            assert!(w.contains(&dir), "{dir:?} watched");
        }
        // hicolor has no 22x22: a 22x22/apps made later shows in hicolor itself.
        assert!(!w.contains(&system.join("hicolor/22x22")));
        assert!(!w.contains(&user.join("test/48x48/apps")));
        assert!(!w.contains(&user.join("base")), "a root without the theme");
        assert_eq!(w.len(), w.iter().collect::<HashSet<_>>().len(), "each once");
        fs::remove_dir_all(system.parent().unwrap()).unwrap();
    }

    #[test]
    fn warming_reads_every_directory_and_keeps_the_answers() {
        let (system, user) = cached_fixture();
        let roots = [user.clone(), system.clone()];
        let (cold, warm) = (
            IconTheme::load("test", &roots),
            IconTheme::load("test", &roots),
        );
        warm.warm();
        assert!(warm.listings.iter().all(|l| l.get().is_some()));
        for name in NAMES {
            for (size, scale) in [(16, 1), (22, 1), (48, 1), (16, 2)] {
                assert_eq!(
                    warm.lookup(name, size, scale),
                    cold.lookup(name, size, scale)
                );
            }
        }
        fs::remove_dir_all(system.parent().unwrap()).unwrap();
    }

    #[test]
    fn only_directories_that_exist_and_changed_since_the_cache_are_listed() {
        let (system, user) = cached_fixture();
        let t = IconTheme::load("test", &[user.clone(), system.clone()]);
        assert!(t.lookup("nothing", 16, 1).is_none());
        let listing = |dir: &Path| {
            let i = t.dirs.iter().position(|d| d.path == dir).unwrap();
            t.listings[i].get().unwrap()
        };
        let names = |dir: &Path| match listing(dir) {
            Listing::Names(n) => {
                let mut n: Vec<_> = n.iter().map(|f| f.to_str().unwrap()).collect();
                n.sort();
                Some(n)
            }
            Listing::Cached(_) => None,
        };
        assert!(names(&system.join("test/16x16/apps")).is_none());
        assert!(names(&system.join("hicolor/scalable/apps")).is_none());
        assert_eq!(
            names(&system.join("test/22x22/apps")),
            Some(vec!["new.png"])
        );
        assert_eq!(
            names(&system.join("hicolor/48x48/apps")),
            Some(vec!["h.png", "late.png"])
        );
        assert_eq!(
            names(&system.join("hicolor/16x16/apps")),
            Some(vec!["h.png"])
        );
        assert_eq!(names(&user.join("test/16x16/apps")), Some(vec!["u.png"]));
        assert!(
            !t.dirs.iter().any(|d| d.path.starts_with(user.join("base"))),
            "a root without the theme adds none of its directories"
        );
        let base = t
            .homes
            .iter()
            .find(|h| h.path == system.join("base"))
            .unwrap();
        let HomeState::Listed(entries) = base.state() else {
            panic!("base has no cache");
        };
        let mut above: Vec<String> = entries
            .lock()
            .unwrap()
            .keys()
            .map(|k| k.to_str().unwrap().to_string())
            .collect();
        above.sort();
        assert_eq!(
            above,
            ["", "16x16", "32x32", "scalable"],
            "only parents that exist"
        );
        fs::remove_dir_all(system.parent().unwrap()).unwrap();
    }
}
