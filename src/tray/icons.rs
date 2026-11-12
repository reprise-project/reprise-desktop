use super::{TrayCommand, TrayItem, builtin, send};
use crate::compositor::State;
use crate::icontheme::{
    ExtraTree, IconCache, MAX_ICON_BYTES, MAX_ICON_DIMENSION, bare_name, decode_bytes,
    is_icon_file, list_at_most,
};
use crate::mutex::lock;
use crate::raster::Canvas;
use reprisefs::confined::Confined;
use reprisekit::controls::shellview;
use reprisekit::icon::{IconSource, premultiply};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Largest `IconPixmap` side read; the whole property must stay under 4 MiB, so a 1024 px
/// entry never arrives. Electron apps send only 512 px.
const MAX_PIXMAP_SIDE: i32 = MAX_ICON_DIMENSION as i32;

/// For each of `sizes`, the px the icon is drawn at ([`icon_sizes`]), the smallest
/// `IconPixmap` entry at least that wide, else the widest, shrunk to the widest size it serves.
pub fn item_pixmaps(raw: &[(i32, i32, &[u8])], sizes: &[i32]) -> Vec<IconSource> {
    let usable = || {
        raw.iter()
            .enumerate()
            .filter(|(_, (w, h, b))| pixmap_len(*w, *h, b.len()).is_some())
    };
    let mut picked: Vec<(usize, i32)> = Vec::with_capacity(sizes.len());
    for &size in sizes {
        let fit = usable()
            .filter(|(_, (w, ..))| *w >= size)
            .min_by_key(|(_, (w, ..))| *w);
        let Some((i, _)) = fit.or_else(|| usable().max_by_key(|(_, (w, ..))| *w)) else {
            continue;
        };
        match picked.iter_mut().find(|(p, _)| *p == i) {
            Some((_, served)) => *served = (*served).max(size),
            None => picked.push((i, size)),
        }
    }
    if picked.is_empty() && !raw.is_empty() {
        log::warn!("tray: no usable IconPixmap among {} entries", raw.len());
    }
    picked
        .into_iter()
        .filter_map(|(i, served)| {
            let source = pixmap_to_source(raw[i].0, raw[i].1, raw[i].2)?;
            let wider = source.width.max(source.height) > u32::try_from(served).ok()?;
            Some(match wider {
                true => IconSource::from(source.to_canvas(served)),
                false => source,
            })
        })
        .collect()
}

/// Byte length of a `w`x`h` ARGB32 pixmap, or `None` for a non-positive
/// side, a side past [`MAX_PIXMAP_SIDE`] (an SNI peer controls `w`/`h`), or
/// a buffer of `have` bytes too short for it.
fn pixmap_len(w: i32, h: i32, have: usize) -> Option<usize> {
    if !(1..=MAX_PIXMAP_SIDE).contains(&w) || !(1..=MAX_PIXMAP_SIDE).contains(&h) {
        return None;
    }
    let len = w as usize * h as usize * 4;
    (have >= len).then_some(len)
}

/// Convert one `IconPixmap` entry (ARGB32, big-endian per pixel, straight
/// alpha) into premultiplied source pixels; `None` where [`pixmap_len`]
/// rejects it.
pub(super) fn pixmap_to_source(w: i32, h: i32, argb_be: &[u8]) -> Option<IconSource> {
    let len = pixmap_len(w, h, argb_be.len())?;
    let pixels = argb_be[..len]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| premultiply(u32::from_be_bytes(*p)))
        .collect();
    Some(IconSource {
        width: w as u32,
        height: h as u32,
        pixels,
    })
}

/// Sizes a notification's icons are decoded at; 48 and 96 px feed the Glass sheet's icon slot.
pub const NOTIFY_SIZES: [i32; 4] = [16, 32, 48, 96];
/// Sizes an action's icon is decoded at: its button's 16 px at scale 1 and 2.
pub const ACTION_ICON_SIZES: [i32; 2] = [16, 32];

/// A peer-named icon (a theme name or an absolute path, with `extra`
/// directories searched first) decoded at each of `sizes`. Only the D-Bus
/// thread calls this: the file is the peer's choice, and even an SVG inside
/// the decoder's budget can take a second to draw.
pub fn decode_named_at(
    icons: &mut IconCache,
    name: &str,
    extra: &[PathBuf],
    sizes: &[i32],
) -> Vec<IconSource> {
    sizes
        .iter()
        .filter_map(|&px| icons.get_with_extra(name, px, extra))
        .map(IconSource::from)
        .collect()
}

/// Where an item's named icon files open from, by where its program runs.
#[derive(Debug)]
pub enum IconFiles {
    /// By path, as the user opens them.
    Host,
    /// Only where its sandbox lets the program open them itself.
    Confined(Confined),
    /// Nowhere: a name is looked up in the theme only.
    Refused,
}

impl IconFiles {
    /// An SVG from a sandbox may not embed a raster image; a host's draws, the image left blank.
    fn decode(&self, path: &Path, bytes: &[u8], px: u32) -> Option<Canvas> {
        decode_bytes(path, bytes, px, matches!(self, IconFiles::Host))
    }

    fn list(&self, dir: &Path, max: usize) -> HashSet<OsString> {
        match self {
            IconFiles::Host => list_at_most(dir, max),
            IconFiles::Confined(files) => files.list(dir, max),
            IconFiles::Refused => HashSet::new(),
        }
    }
}

/// The most one decode of an item's names reads of the files they name, all three together.
pub const FETCH_BYTES: u64 = 2 * MAX_ICON_BYTES;

/// One decode's reads of an item's files, [`FETCH_BYTES`] in all.
struct Reads<'a> {
    files: &'a IconFiles,
    left: u64,
}

impl Reads<'_> {
    fn read(&mut self, path: &Path) -> Option<Vec<u8>> {
        if !is_icon_file(path) {
            return None;
        }
        let max = self.left.min(MAX_ICON_BYTES);
        let read = match self.files {
            IconFiles::Host => reprisefs::ops::read_regular(path, max),
            IconFiles::Confined(files) => files.read(path, max),
            IconFiles::Refused => return None,
        };
        match read {
            Ok(bytes) => {
                self.left -= bytes.len() as u64;
                Some(bytes)
            }
            Err(e) => {
                log::debug!("tray: icon {}: {e}", path.display());
                None
            }
        }
    }
}

/// An item's icon, attention and overlay `names` (None where pixmaps stand in) at each of
/// `sizes`, their files read through `files`; `icons` is locked only for theme lookups.
pub fn decode_item_icons(
    icons: &Mutex<IconCache>,
    files: &IconFiles,
    names: [Option<&str>; 3],
    theme_path: Option<&Path>,
    sizes: &[i32],
) -> [Vec<IconSource>; 3] {
    let mut reads = Reads {
        files,
        left: FETCH_BYTES,
    };
    // One tree for the three names, so its folder cap holds for the item.
    let mut tree = theme_path.map(|dir| ExtraTree::new(dir, |d: &Path, max| files.list(d, max)));
    names.map(|name| {
        name.map_or_else(Vec::new, |name| {
            decode_item_icon(icons, &mut reads, name, tree.as_mut(), sizes)
        })
    })
}

fn decode_item_icon<F: FnMut(&Path, usize) -> HashSet<OsString>>(
    icons: &Mutex<IconCache>,
    reads: &mut Reads,
    name: &str,
    tree: Option<&mut ExtraTree<F>>,
    sizes: &[i32],
) -> Vec<IconSource> {
    let files = reads.files;
    if name.starts_with('/') {
        let path = Path::new(name);
        let Some(bytes) = reads.read(path) else {
            return Vec::new();
        };
        return (sizes.iter())
            .filter_map(|&px| files.decode(path, &bytes, u32::try_from(px).ok()?))
            .map(IconSource::from)
            .collect();
    }
    let bare = bare_name(name);
    let mut tree = tree.filter(|_| !bare.contains('/'));
    // The bytes of each file read, kept for the other sizes only while it decodes.
    let mut read: HashMap<PathBuf, Option<Vec<u8>>> = HashMap::new();
    let mut out = Vec::with_capacity(sizes.len());
    for &px in sizes {
        let own = (tree.as_mut().zip(u32::try_from(px).ok())).and_then(|(tree, dpx)| {
            tree.files(bare, px).into_iter().find_map(|path| {
                let bytes = read
                    .entry(path.clone())
                    .or_insert_with(|| reads.read(&path));
                let decoded = files.decode(&path, bytes.as_deref()?, dpx);
                if decoded.is_none() {
                    *bytes = None;
                }
                decoded
            })
        });
        let found = own.or_else(|| lock(icons).get_with_extra(name, px, &[]));
        out.extend(found.map(IconSource::from));
    }
    out
}

pub fn closest(sources: &[IconSource], px: i32) -> Option<Canvas> {
    sources
        .iter()
        .min_by_key(|s| (s.width as i32 - px).abs())
        .map(|s| s.to_canvas(px))
}

/// The px an item's icon is drawn at, each once and ascending: a tray cell's 16 px and the
/// Customize list's `list` px, at each draw scale in `scales`.
pub fn icon_sizes(scales: &[i32], list: i32) -> Vec<i32> {
    let mut sizes: Vec<i32> = scales
        .iter()
        .flat_map(|&s| [reprisekit::icon::ICON_SIZE * s, list * s])
        .collect();
    sizes.sort_unstable();
    sizes.dedup();
    sizes
}

/// [`icon_sizes`] at the outputs' draw scales (1 with no output) and the text size's list icon.
pub(super) fn drawn_sizes(state: &State) -> Vec<i32> {
    let mut scales: Vec<i32> = (state.scene.outputs().iter())
        .map(|o| state.draw_scale_at(Some(o)))
        .collect();
    if scales.is_empty() {
        scales.push(1);
    }
    let list = shellview::list_icon_px(state.config.theme.controls().text);
    icon_sizes(&scales, list)
}

/// After a text size or an output change: has the D-Bus thread decode item icons at the new
/// [`drawn_sizes`], when they differ from the ones it has.
pub fn sync_icon_sizes(state: &mut State) {
    let sizes = drawn_sizes(state);
    let Some(tray) = state.tray.as_mut() else {
        return;
    };
    if tray.icon_sizes != sizes {
        tray.icon_sizes = sizes.clone();
        send(state, TrayCommand::IconSizes(sizes));
    }
}

/// `item`'s icon at `px`: its attention icon while it needs attention and has one, else its own,
/// with its overlay drawn over the bottom-right quarter. None when it has no icon.
pub fn item_canvas(item: &TrayItem, px: i32) -> Option<Canvas> {
    let attention = &item.attention;
    let wants =
        item.needs_attention && !(attention.pixmaps.is_empty() && attention.named.is_empty());
    let (pixmaps, named) = if wants {
        (&attention.pixmaps, &attention.named)
    } else {
        (&item.pixmaps, &item.named)
    };
    let mut icon = closest(pixmaps, px).or_else(|| closest(named, px))?;
    let side = px / 2;
    let overlay = (side > 0)
        .then(|| {
            closest(&item.overlay.pixmaps, side).or_else(|| closest(&item.overlay.named, side))
        })
        .flatten();
    if let Some(o) = overlay {
        let (w, h) = (icon.width as i32, icon.height as i32);
        icon.blit_over(w - o.width as i32, h - o.height as i32, &o);
    }
    Some(icon)
}

pub fn item_icon(state: &mut State, item: &TrayItem, px: i32) -> Option<Arc<Canvas>> {
    let key = (item.id.clone(), px, item.needs_attention);
    if let Some(c) = state.tray.as_ref().and_then(|t| t.icon_cache.get(&key)) {
        return c.clone();
    }
    let waits = state.icons.waits();
    let found = item_canvas(item, px)
        .map(Arc::new)
        .or_else(|| crate::icon::unread(&mut state.icons, crate::icon::STAND_IN, px));
    // A stand-in still decoding is asked for again when the bar draws once it lands.
    let settled = state.icons.waits() == waits;
    if let Some(t) = state.tray.as_mut().filter(|_| settled) {
        t.icon_cache.insert(key, found.clone());
    }
    found
}

pub fn bar_icons(state: &mut State, px: i32, revealed: usize) -> Vec<Option<Arc<Canvas>>> {
    let open: Vec<TrayItem> = state.tray.as_ref().map_or_else(Vec::new, |t| {
        let from = t.hidden.len().saturating_sub(revealed);
        t.hidden[from..].to_vec()
    });
    let items: Vec<TrayItem> = state
        .tray
        .as_ref()
        .map(|t| t.shown().to_vec())
        .unwrap_or_default();
    let (lead, trail): (Vec<_>, Vec<_>) =
        builtin::shown(state).into_iter().partition(|b| b.leads());
    let own = |state: &mut State, b| builtin::icon(state, b, px).map(Arc::new);
    let mut icons: Vec<Option<Arc<Canvas>>> =
        open.iter().map(|it| item_icon(state, it, px)).collect();
    icons.extend(lead.into_iter().map(|b| own(state, b)));
    icons.extend(items.iter().map(|it| item_icon(state, it, px)));
    icons.extend(trail.into_iter().map(|b| own(state, b)));
    icons
}

pub fn invalidate_icons(state: &mut State) {
    if let Some(t) = state.tray.as_mut() {
        t.icon_cache.clear();
    }
}
