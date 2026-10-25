//! freedesktop icon theme lookup: `index.theme` parsing, the inherits chain and the size-matching
//! rules from the Icon Theme Specification, read through each theme's current `icon-theme.cache`.

mod cache;
mod decode;
mod index;
mod listing;
#[cfg(test)]
mod tests;

pub use cache::{IconCache, grown_icon, load_icon};
pub(crate) use decode::png_rgba;
pub use decode::{MAX_ICON_DIMENSION, canvas_from_rgba, decode, decode_file, png_to_canvas};
pub use index::{DirEntry, DirKind};
pub use listing::{LISTING_PX, ListingIcons, ListingSizes, listing_px};

use crate::mutex::lock;
use crate::themecache::{self, ThemeCache};
use index::parse_index;
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Env var carrying the icon theme name from the compositor; clients without it load `hicolor`.
pub const ENV: &str = "REPRISE_ICONS";

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
        let mut entries = lock(entries);
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
        let mut unchanged = lock(unchanged);
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

    /// The directories whose entries lookups depend on: the roots, made or not, each theme's
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
        let roots = self.roots.iter().map(PathBuf::as_path);
        for dir in roots.chain(homes).chain(sizes) {
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
        let mut orders = lock(&self.orders);
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
