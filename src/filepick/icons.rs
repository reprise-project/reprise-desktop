use super::*;
use reprisekit::art::viewsicon;
use reprisekit::controls::placesbar::{self, Place};
use reprisekit::icontheme::{self, IconCache, IconTheme};
use reprisekit::theme::TextUnits;
use std::sync::Arc;

/// The tiles for `tiles`, pinned ones wearing `icons.pinned`.
pub(super) fn tile_places(tiles: &[reprisefs::places::DialogPlace], icons: &Icons) -> Vec<Place> {
    (tiles.iter().zip(tile_icons(tiles, icons)))
        .map(|(p, icon)| Place {
            enabled: p.dir.is_some(),
            label: p.label.clone(),
            icon,
            pinned: p.pin.is_some(),
        })
        .collect()
}

pub(super) fn tile_icons(
    tiles: &[reprisefs::places::DialogPlace],
    icons: &Icons,
) -> Vec<Option<Canvas>> {
    (tiles.iter().enumerate())
        .map(|(i, p)| match p.pin {
            Some(_) => icons.pinned.clone(),
            None => icons.places.get(i).cloned().flatten(),
        })
        .collect()
}

/// The dialog's own icons, fetched by the host: the places tiles at 32 px in
/// `reprisefs::places::dialog_places`' order, the toolbar's buttons at 16 in `TOOLBAR`'s and the
/// Look in folder at 16, all grown with the text size. The dialog draws on device pixels, so each
/// is a scale-1 canvas `scale` times that size, as are the listing's, which come from the cache
/// handed to [`FilePicker::reply`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Icons {
    pub places: Vec<Option<Canvas>>,
    /// A pinned folder's tile icon.
    pub pinned: Option<Canvas>,
    pub toolbar: Vec<Option<Canvas>>,
    pub folder: Option<Canvas>,
    pub scale: i32,
    pub text: TextUnits,
    pub theme: ThemeOf,
}

/// The icon theme a dialog's icons came from: equal only to the same loaded theme.
#[derive(Clone, Default)]
pub struct ThemeOf(Option<Arc<IconTheme>>);

impl ThemeOf {
    pub fn of(cache: &IconCache) -> ThemeOf {
        ThemeOf(Some(cache.theme()))
    }
}

impl PartialEq for ThemeOf {
    fn eq(&self, o: &ThemeOf) -> bool {
        match (&self.0, &o.0) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (a, b) => a.is_none() && b.is_none(),
        }
    }
}

impl std::fmt::Debug for ThemeOf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_ref().map_or("no theme", |_| "a theme"))
    }
}

/// The dialog's icons from the compositor's scale-1 `cache`, for a dialog drawn at `scale` with
/// text grown by `u`.
pub fn icons(cache: &mut IconCache, scale: i32, u: TextUnits) -> Icons {
    let scale = scale.max(1);
    let tool_px = u.len(16) * scale;
    let mut get = |name: &str, px: i32| {
        let icon = cache.get(name, px)?;
        Some(icontheme::grown_icon(icon, px, u))
    };
    Icons {
        places: reprisefs::places::dialog_places(true)
            .iter()
            .map(|p| get(p.icon, u.len(placesbar::ICON_PX) * scale))
            .collect(),
        pinned: get("folder", u.len(placesbar::ICON_PX) * scale),
        toolbar: TOOLBAR
            .iter()
            .map(|i| get(i, tool_px))
            .enumerate()
            .map(|(n, icon)| match icon {
                None if n == TOOLBAR_VIEWS => Some(viewsicon::draw16_near(tool_px).at_scale(1)),
                icon => icon,
            })
            .collect(),
        folder: get("folder", tool_px),
        scale,
        text: u,
        theme: ThemeOf::of(cache),
    }
}

/// The icon files `rows` wear, each decoded once at every listing size times `scale`.
pub(super) fn icon_files(
    rows: &[Listed],
    scale: i32,
    list: i32,
) -> Vec<(String, [Option<Canvas>; 3])> {
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
            let art = icontheme::listing_px(list)
                .map(|px| reprisekit::icontheme::decode_file(Path::new(f), px * scale, 1));
            (f.to_string(), art)
        })
        .collect()
}
