//! The icons a folder listing wears, fetched again when its window draws at a new scale.

use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, channel};

use crate::Waker;
use reprisefs::listing::Item;
use reprisekit::controls::shellview::{self, Entry};
use reprisekit::icontheme::{IconCache, LISTING_PX, ListingIcons, decode_file};
use reprisekit::raster::Canvas;

type Art = Vec<(String, [Option<Canvas>; 3])>;

/// Puts `art`, decoded at `scale`, into `cache` in one batch.
fn put_art(cache: &mut IconCache, art: &[(String, [Option<Canvas>; 3])], scale: i32) {
    let puts = art.iter().flat_map(|(file, a)| {
        LISTING_PX
            .into_iter()
            .zip(a)
            .map(|(px, c)| (file.clone(), px, c.clone()))
    });
    cache.put_files_at(puts.collect(), scale);
}

/// What a listing's thread reads for its rows' icons, beside the rows: each folder's own icon
/// (`.directory`'s `Icon=`), and the icon files the rows name decoded at [`LISTING_PX`]. A
/// subfolder may be a mount point, and an icon file may be anywhere, that never answers.
#[derive(Debug, Default)]
pub struct ReadIcons {
    /// Row `i`'s own icon, for `reprisefs::icons::item_icon_with`.
    pub own: Vec<Option<String>>,
    art: Art,
    scale: i32,
}

impl ReadIcons {
    /// Reads `items`' own icons and decodes the icon files they name at `scale`.
    pub fn read<'a>(items: impl IntoIterator<Item = &'a Item>, scale: i32) -> ReadIcons {
        let mut files: Vec<String> = Vec::new();
        let own = items
            .into_iter()
            .map(|it| {
                let own = reprisefs::icons::own_icon(it);
                let named = match &it.shortcut {
                    Some(sc) => Some(sc.icon()),
                    None => own.as_deref(),
                };
                files.extend(named.filter(|n| n.starts_with('/')).map(str::to_string));
                own
            })
            .collect();
        files.sort_unstable();
        files.dedup();
        let art = files
            .into_iter()
            .map(|f| {
                let a = LISTING_PX.map(|px| decode_file(Path::new(&f), px, scale));
                (f, a)
            })
            .collect();
        ReadIcons { own, art, scale }
    }

    /// Hands `cache` the decoded icon files, so fetching the rows' icons reads none of them.
    pub fn put(&self, cache: &mut IconCache) {
        put_art(cache, &self.art, self.scale);
    }
}

/// An Explorer or file dialog listing's icons: a new draw scale swaps the entries' icons without
/// listing the folder again, theme icons at once and icon files once a thread decodes them.
#[derive(Default)]
pub struct ListIcons {
    icons: ListingIcons,
    scale: i32,
    files: Option<Receiver<Art>>,
}

impl ListIcons {
    /// A new listing wears `icons`, fetched from a cache at `scale`.
    pub fn listed(&mut self, icons: ListingIcons, scale: i32) {
        *self = ListIcons {
            icons,
            scale,
            files: None,
        };
    }

    /// `worn`, an icon an entry wears at `px`, from `cache` at the cache's scale: a theme icon
    /// fetched, an icon file only as the cache already holds it. None when the listing does not
    /// know `worn` or the cache has no such file.
    pub fn fetch(&self, worn: &Rc<Canvas>, px: i32, cache: &mut IconCache) -> Option<Canvas> {
        let name = self.icons.name_of(worn)?;
        if name.starts_with('/') {
            cache
                .file_put(name, px)
                .flatten()
                .map(std::sync::Arc::unwrap_or_clone)
        } else {
            cache.get(name, px)
        }
    }

    /// The window draws at `scale`, `cache`'s scale: `entries` wear its theme icons now, and the
    /// icon files `cache` lacks decode on a thread that fires `wake` ([`ListIcons::take`]).
    pub fn follow(
        &mut self,
        scale: i32,
        cache: &mut IconCache,
        entries: &mut [Entry],
        wake: &Waker,
    ) {
        if std::mem::replace(&mut self.scale, scale) == scale {
            return;
        }
        let (swaps, files) = self.icons.refetch(cache);
        shellview::swap_icons(entries, &swaps);
        // Dropping the receiver drops what a thread for the old scale still sends.
        self.files = None;
        if files.is_empty() {
            return;
        }
        let (tx, rx) = channel();
        let wake = wake.clone();
        let spawned = std::thread::Builder::new()
            .name("list icons".into())
            .spawn(move || {
                let art: Art = files
                    .into_iter()
                    .map(|f| {
                        let a = LISTING_PX.map(|px| decode_file(Path::new(&f), px, scale));
                        (f, a)
                    })
                    .collect();
                if tx.send(art).is_ok() {
                    wake.wake();
                }
            });
        match spawned {
            Ok(_) => self.files = Some(rx),
            Err(e) => log::warn!("cannot spawn an icon thread ({e}); icon files keep their scale"),
        }
    }

    /// Hands `entries` the icon files decoded for the last scale change, and `cache` keeps them
    /// at that scale: true when an entry changed. A file that no longer decodes keeps its old
    /// icons.
    pub fn take(&mut self, entries: &mut [Entry], cache: &mut IconCache) -> bool {
        let Some(art) = self.files.as_ref().and_then(|rx| rx.try_recv().ok()) else {
            return false;
        };
        self.files = None;
        let art: Vec<_> = art
            .into_iter()
            .filter(|(_, a)| a.iter().any(Option::is_some))
            .collect();
        put_art(cache, &art, self.scale);
        let swaps: Vec<_> = art
            .into_iter()
            .filter_map(|(f, a)| self.icons.put(&f, a))
            .collect();
        shellview::swap_icons(entries, &swaps);
        !swaps.is_empty()
    }
}
