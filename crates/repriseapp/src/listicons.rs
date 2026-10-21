//! The icons a folder listing wears, fetched again when its window draws at a new scale or text
//! size.

use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, channel};

use crate::Waker;
use reprisefs::listing::Item;
use reprisekit::controls::shellview::{self, Entry};
use reprisekit::icontheme::{IconCache, LISTING_PX, ListingIcons, decode_file};
use reprisekit::raster::Canvas;

/// Icon files decoded at a scale, each at its sizes in logical px.
type Art = Vec<(String, Vec<(i32, Option<Canvas>)>)>;

/// Decodes `files` at each of `sizes` and `scale`.
fn decode_art(files: Vec<String>, sizes: &[i32], scale: i32) -> Art {
    files
        .into_iter()
        .map(|f| {
            let a = (sizes.iter())
                .map(|&px| (px, decode_file(Path::new(&f), px, scale)))
                .collect();
            (f, a)
        })
        .collect()
}

/// Puts `art`, decoded at `scale`, into `cache` in one batch.
fn put_art(cache: &mut IconCache, art: &Art, scale: i32) {
    let puts = art
        .iter()
        .flat_map(|(file, a)| a.iter().map(|(px, c)| (file.clone(), *px, c.clone())));
    cache.put_files_at(puts.collect(), scale);
}

/// Decodes `files` at `sizes` and `scale` on a thread that fires `wake` once the art is sent.
/// None when no thread starts.
fn spawn_decode(
    files: Vec<String>,
    sizes: Vec<i32>,
    scale: i32,
    wake: &Waker,
) -> Option<Receiver<Art>> {
    let (tx, rx) = channel();
    let wake = wake.clone();
    let spawned = std::thread::Builder::new()
        .name("list icons".into())
        .spawn(move || {
            let art = decode_art(files, &sizes, scale);
            if tx.send(art).is_ok() {
                wake.wake();
            }
        });
    match spawned {
        Ok(_) => Some(rx),
        Err(e) => {
            log::warn!("cannot spawn an icon thread ({e}); icon files keep their scale");
            None
        }
    }
}

/// What a listing's thread reads for its rows' icons, beside the rows: each folder's own icon
/// (`.directory`'s `Icon=`), and the icon files the rows name decoded at the listing's sizes. A
/// subfolder may be a mount point, and an icon file may be anywhere, that never answers.
#[derive(Debug, Default)]
pub struct ReadIcons {
    /// Row `i`'s own icon, for `reprisefs::icons::item_icon_with`.
    pub own: Vec<Option<String>>,
    art: Art,
    scale: i32,
}

impl ReadIcons {
    /// Reads `items`' own icons and decodes the icon files they name at `sizes`
    /// ([`ListingIcons::sizes`]) and `scale`.
    pub fn read<'a>(
        items: impl IntoIterator<Item = &'a Item>,
        sizes: [i32; 3],
        scale: i32,
    ) -> ReadIcons {
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
        let art = decode_art(files, &sizes, scale);
        ReadIcons { own, art, scale }
    }

    /// Hands `cache` the decoded icon files, so fetching the rows' icons reads none of them.
    pub fn put(&self, cache: &mut IconCache) {
        put_art(cache, &self.art, self.scale);
    }
}

/// An Explorer or file dialog listing's icons: a new draw scale or List and Details icon size
/// swaps the entries' icons without listing the folder again, theme icons at once and icon files
/// once a thread decodes them.
#[derive(Default)]
pub struct ListIcons {
    icons: ListingIcons,
    scale: i32,
    files: Option<Receiver<Art>>,
    /// Icon files [`ListIcons::fetch`] found missing at a scale, waiting for [`ListIcons::decode_misses`].
    misses: Vec<String>,
    /// The scale and decoded art of the last [`ListIcons::decode_misses`].
    decoding: Option<(i32, Receiver<Art>)>,
}

impl ListIcons {
    /// A new listing wears `icons`, fetched from a cache at `scale`.
    pub fn listed(&mut self, icons: ListingIcons, scale: i32) {
        *self = ListIcons {
            icons,
            scale,
            ..ListIcons::default()
        };
    }

    /// `worn`, an icon an entry wears at `px`, from `cache` at the cache's scale: a theme icon
    /// fetched, an icon file only as the cache already holds it. None when the listing does not
    /// know `worn` or the cache has no such file; a file never put at this scale is kept for
    /// [`ListIcons::decode_misses`].
    pub fn fetch(&mut self, worn: &Rc<Canvas>, px: i32, cache: &mut IconCache) -> Option<Canvas> {
        let name = self.icons.name_of(worn)?;
        if !name.starts_with('/') {
            return cache.get(name, px);
        }
        match cache.file_put(name, px) {
            Some(put) => put.map(std::sync::Arc::unwrap_or_clone),
            None => {
                if !self.misses.iter().any(|m| m == name) {
                    self.misses.push(name.to_string());
                }
                None
            }
        }
    }

    /// Decodes the icon files `fetch` missed at `scale` on a thread that fires `wake`
    /// ([`ListIcons::take_misses`]), in place of a decode still running.
    pub fn decode_misses(&mut self, scale: i32, wake: &Waker) {
        let files = std::mem::take(&mut self.misses);
        if files.is_empty() {
            return;
        }
        let mut sizes: Vec<i32> = LISTING_PX.into_iter().chain(self.icons.sizes()).collect();
        sizes.sort_unstable();
        sizes.dedup();
        self.decoding = spawn_decode(files, sizes, scale, wake).map(|rx| (scale, rx));
    }

    /// Hands `cache` the icon files [`ListIcons::decode_misses`] decoded, at the scale they were
    /// decoded at: true when they arrived.
    pub fn take_misses(&mut self, cache: &mut IconCache) -> bool {
        let Some((scale, art)) = self
            .decoding
            .as_ref()
            .and_then(|(s, rx)| rx.try_recv().ok().map(|a| (*s, a)))
        else {
            return false;
        };
        self.decoding = None;
        put_art(cache, &art, scale);
        true
    }

    /// The view now draws the `drawn` sizes ([`shellview::drawn_sizes`]): `entries` wear the theme
    /// icons fetched at those they lacked before the next frame.
    pub fn draw(&mut self, drawn: [bool; 3], cache: &mut IconCache, entries: &mut [Entry]) {
        let swaps = self.icons.draw(cache, drawn);
        shellview::swap_icons(entries, &swaps);
    }

    /// The window draws at `scale`, `cache`'s scale, with List and Details' icons `list` px
    /// ([`shellview::list_icon_px`]): `entries` wear its theme icons now, and the icon files
    /// `cache` lacks decode on a thread that fires `wake` ([`ListIcons::take`]).
    pub fn follow(
        &mut self,
        scale: i32,
        list: i32,
        cache: &mut IconCache,
        entries: &mut [Entry],
        wake: &Waker,
    ) {
        let relisted = self.icons.set_list(list);
        if std::mem::replace(&mut self.scale, scale) == scale && !relisted {
            return;
        }
        let (swaps, files) = self.icons.refetch(cache);
        shellview::swap_icons(entries, &swaps);
        // Dropping the receiver drops what a thread for the old scale still sends.
        self.files = None;
        if files.is_empty() {
            return;
        }
        self.files = spawn_decode(files, self.icons.sizes().to_vec(), scale, wake);
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
            .filter(|(_, a)| a.iter().any(|(_, c)| c.is_some()))
            .collect();
        put_art(cache, &art, self.scale);
        let sizes = self.icons.sizes();
        let swaps: Vec<_> = art
            .into_iter()
            .filter_map(|(f, a)| {
                let at = |px| {
                    a.iter()
                        .find(|(p, _)| *p == px)
                        .and_then(|(_, c)| c.clone())
                };
                self.icons.put(&f, sizes.map(at))
            })
            .collect();
        shellview::swap_icons(entries, &swaps);
        !swaps.is_empty()
    }
}
