use super::{TrayCommand, TrayItem, builtin, send};
use crate::compositor::State;
use crate::icontheme::IconCache;
use crate::raster::Canvas;
use reprisekit::controls::shellview;
use reprisekit::icon::{IconSource, premultiply};
use std::path::PathBuf;
use std::sync::Arc;

/// Largest `IconPixmap` side the tray keeps. It draws at most 32 px per step of
/// draw scale, so a bigger entry only costs memory.
const MAX_PIXMAP_SIDE: i32 = 256;

/// For each of `sizes`, the px the icon is drawn at ([`icon_sizes`]), the smallest
/// `IconPixmap` entry at least that wide, else the widest: drawing shrinks it.
pub fn item_pixmaps(raw: &[(i32, i32, &[u8])], sizes: &[i32]) -> Vec<IconSource> {
    let usable = || {
        raw.iter()
            .enumerate()
            .filter(|(_, (w, h, b))| pixmap_len(*w, *h, b.len()).is_some())
    };
    let mut picked: Vec<usize> = Vec::with_capacity(sizes.len());
    for &size in sizes {
        let fit = usable()
            .filter(|(_, (w, ..))| *w >= size)
            .min_by_key(|(_, (w, ..))| *w);
        if let Some((i, _)) = fit.or_else(|| usable().max_by_key(|(_, (w, ..))| *w))
            && !picked.contains(&i)
        {
            picked.push(i);
        }
    }
    if picked.is_empty() && !raw.is_empty() {
        log::warn!("tray: no usable IconPixmap among {} entries", raw.len());
    }
    picked
        .into_iter()
        .filter_map(|i| pixmap_to_source(raw[i].0, raw[i].1, raw[i].2))
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
    let key = (item.id.clone(), px);
    if let Some(c) = state.tray.as_ref().and_then(|t| t.icon_cache.get(&key)) {
        return c.clone();
    }
    let found = item_canvas(item, px)
        .or_else(|| state.icons.get("application-x-executable", px))
        .map(Arc::new);
    if let Some(t) = state.tray.as_mut() {
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
