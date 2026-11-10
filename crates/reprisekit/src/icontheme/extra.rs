use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// Extensions looked for in an extra folder: those [`super::decode`] reads.
const EXTENSIONS: [&str; 2] = ["png", "svg"];

/// The most folders one tree lists; past it a folder shows no entries.
pub const EXTRA_MAX_FOLDERS: usize = 64;
/// The most entry names one listing keeps.
pub const EXTRA_MAX_ENTRIES: usize = 1024;
/// The most files one [`ExtraTree::files`] returns.
pub const EXTRA_MAX_FILES: usize = 32;

/// An extra icon folder (a tray item's `IconThemePath`), searched as the base of a `hicolor`
/// tree and then flat. `list(dir, max)` gives at most `max` entry names, once per folder.
pub struct ExtraTree<F> {
    base: PathBuf,
    list: F,
    listed: HashMap<PathBuf, HashSet<OsString>>,
    unlisted: HashSet<OsString>,
}

/// A `hicolor` size folder: its device px (`NxN@S` is N*S), or None for `scalable`.
type SizeDir = (OsString, Option<i32>);

impl<F: FnMut(&Path, usize) -> HashSet<OsString>> ExtraTree<F> {
    pub fn new(base: &Path, list: F) -> Self {
        ExtraTree {
            base: base.to_path_buf(),
            list,
            listed: HashMap::new(),
            unlisted: HashSet::new(),
        }
    }

    fn entries(&mut self, dir: &Path) -> &HashSet<OsString> {
        if !self.listed.contains_key(dir) && self.listed.len() >= EXTRA_MAX_FOLDERS {
            return &self.unlisted;
        }
        let list = &mut self.list;
        self.listed
            .entry(dir.to_path_buf())
            .or_insert_with(|| list(dir, EXTRA_MAX_ENTRIES))
    }

    /// Up to [`EXTRA_MAX_FILES`] files `name` (no extension) may be for `px`, best first:
    /// `hicolor/<size>/<context>/` exact, `scalable`, closest bigger, closest smaller; then flat.
    pub fn files(&mut self, name: &str, px: i32) -> Vec<PathBuf> {
        let files = EXTENSIONS.map(|ext| OsString::from(format!("{name}.{ext}")));
        let base = self.base.clone();
        let found_in = |tree: &mut Self, dir: &Path| -> Vec<PathBuf> {
            let names = tree.entries(dir);
            (files.iter().filter(|f| names.contains(*f)))
                .map(|f| dir.join(f))
                .collect()
        };
        // Listed first so the tree's folders can't use up the listings before it.
        let flat = found_in(self, &base);
        let hicolor = base.join("hicolor");
        let mut sizes: Vec<SizeDir> = (self.entries(&hicolor).iter())
            .filter_map(|d| Some((d.clone(), size_of_dir(d)?)))
            .collect();
        sizes.sort_by_key(|(d, size)| (rank(*size, px), d.clone()));
        let mut out = Vec::new();
        'walk: for (size, _) in sizes {
            let at = hicolor.join(size);
            let mut contexts: Vec<OsString> = self.entries(&at).iter().cloned().collect();
            contexts.sort();
            for context in contexts {
                out.extend(found_in(self, &at.join(context)));
                if out.len() >= EXTRA_MAX_FILES || self.listed.len() >= EXTRA_MAX_FOLDERS {
                    break 'walk;
                }
            }
        }
        out.extend(flat);
        out.truncate(EXTRA_MAX_FILES);
        out
    }
}

/// `Some(Some(px))` for `NxN` or `NxN@S`, `Some(None)` for `scalable`, None for anything else.
fn size_of_dir(dir: &OsStr) -> Option<Option<i32>> {
    let dir = dir.to_str()?;
    if dir == "scalable" {
        return Some(None);
    }
    let (size, scale) = dir.split_once('@').unwrap_or((dir, "1"));
    let (w, h) = size.split_once('x')?;
    let (w, h, scale): (i32, i32, i32) = (w.parse().ok()?, h.parse().ok()?, scale.parse().ok()?);
    (w == h && w > 0 && scale > 0)
        .then(|| w.checked_mul(scale))
        .flatten()
        .map(Some)
}

/// Where a size folder of `size` device px (None: scalable) goes in the search for `px`.
fn rank(size: Option<i32>, px: i32) -> (u8, i32) {
    match size {
        Some(s) if s == px => (0, 0),
        None => (1, 0),
        Some(s) if s > px => (2, s - px),
        Some(s) => (3, px - s),
    }
}
